/**
 * Binary WebSocket PTY framing & watermark ACK flow control aligned with `stratad/src/web/pty_session.rs`.
 *
 * Frame layout:
 * - `0x00` + raw PTY bytes (stdin client->server, stdout/stderr server->client)
 * - `0x01` + UTF-8 JSON `{ cols, rows }` (client->server resize)
 * - `0x02` + 4-byte unsigned big-endian byte count (client->server flow-control ACK)
 * - `0x03` + UTF-8 JSON `{ exitCode, signal? }` (server->client process exit)
 */

export const PTY_FRAME_DATA = 0x00;
export const PTY_FRAME_RESIZE = 0x01;
export const PTY_FRAME_ACK = 0x02;
export const PTY_FRAME_EXIT = 0x03;

/** Send an ACK frame every 64 KiB consumed by xterm.js. */
export const PTY_ACK_BATCH_BYTES = 64 * 1024;

/** Client-side high watermark warning threshold (256 KiB). */
export const PTY_HIGH_WATERMARK_BYTES = 256 * 1024;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export interface PtyExitEvent {
  exitCode: number;
  signal?: number;
}

export interface PtySessionMeta {
  sessionId: string;
  container: string;
  pid?: number;
}

export type DecodedServerPtyFrame =
  | { type: 'data'; payload: Uint8Array }
  | { type: 'exit'; exitCode: number; signal?: number }
  | { type: 'unknown'; opcode: number };

/** Encodes raw UTF-8 string or bytes into a `0x00` PTY data frame. */
export function encodePtyDataFrame(input: string | Uint8Array): Uint8Array<ArrayBuffer> {
  const body = typeof input === 'string' ? encoder.encode(input) : input;
  const out = new Uint8Array(new ArrayBuffer(1 + body.length));
  out[0] = PTY_FRAME_DATA;
  out.set(body, 1);
  return out;
}

/** Encodes terminal dimensions into a `0x01` JSON resize frame. */
export function encodePtyResizeFrame(cols: number, rows: number): Uint8Array<ArrayBuffer> {
  const safeCols = Math.max(2, Math.min(1000, Math.floor(cols)));
  const safeRows = Math.max(2, Math.min(500, Math.floor(rows)));
  const json = encoder.encode(JSON.stringify({ cols: safeCols, rows: safeRows }));
  const out = new Uint8Array(new ArrayBuffer(1 + json.length));
  out[0] = PTY_FRAME_RESIZE;
  out.set(json, 1);
  return out;
}

/** Encodes acknowledged bytes into a `0x02` u32 big-endian flow-control ACK frame. */
export function encodePtyAckFrame(ackedBytes: number): Uint8Array<ArrayBuffer> {
  const clamped = Math.max(0, Math.min(0xffff_ffff, Math.floor(ackedBytes)));
  const out = new Uint8Array(new ArrayBuffer(5));
  out[0] = PTY_FRAME_ACK;
  const view = new DataView(out.buffer);
  view.setUint32(1, clamped, false);
  return out;
}

/** Decodes a binary server->client PTY WebSocket frame. */
export function decodeServerPtyFrame(raw: ArrayBuffer | Uint8Array): DecodedServerPtyFrame {
  const bytes = raw instanceof Uint8Array ? raw : new Uint8Array(raw);
  if (bytes.length === 0) {
    return { type: 'unknown', opcode: -1 };
  }
  const opcode = bytes[0];
  const body = bytes.subarray(1);
  if (opcode === PTY_FRAME_DATA) {
    return { type: 'data', payload: body };
  }
  if (opcode === PTY_FRAME_EXIT) {
    try {
      const parsed = JSON.parse(decoder.decode(body)) as Partial<PtyExitEvent>;
      return {
        type: 'exit',
        exitCode: typeof parsed.exitCode === 'number' ? parsed.exitCode : 0,
        ...(typeof parsed.signal === 'number' ? { signal: parsed.signal } : {}),
      };
    } catch {
      return { type: 'exit', exitCode: 0 };
    }
  }
  return { type: 'unknown', opcode };
}

/** Parses the initial JSON text handshake sent by `stratad` on `/ws/pty/:id`. */
export function parsePtySessionMeta(text: string): PtySessionMeta | null {
  try {
    const parsed = JSON.parse(text) as Partial<PtySessionMeta>;
    if (typeof parsed.sessionId === 'string' && typeof parsed.container === 'string') {
      return {
        sessionId: parsed.sessionId,
        container: parsed.container,
        ...(typeof parsed.pid === 'number' ? { pid: parsed.pid } : {}),
      };
    }
  } catch {
    // Ignore non-JSON control text frames
  }
  return null;
}

/** Snapshot of client-side PTY watermark flow control metrics. */
export interface PtyFlowStats {
  inFlightBytes: number;
  totalReceivedBytes: number;
  totalAckedBytes: number;
  backpressured: boolean;
}

/**
 * Tracks bytes received from the server vs. committed by `xterm.write(chunk, cb)`
 * and emits batched `0x02` ACK frames every `batchBytes` (default 64 KiB).
 */
export class PtyWatermarkController {
  private readonly batchBytes: number;
  private readonly highWatermarkBytes: number;
  private inFlight = 0;
  private pendingAck = 0;
  private totalReceived = 0;
  private totalAcked = 0;

  constructor(batchBytes = PTY_ACK_BATCH_BYTES, highWatermarkBytes = PTY_HIGH_WATERMARK_BYTES) {
    this.batchBytes = batchBytes;
    this.highWatermarkBytes = highWatermarkBytes;
  }

  /** Records incoming data bytes before scheduling `xterm.write`. */
  onReceive(byteLength: number): boolean {
    if (byteLength <= 0) return this.isBackpressured();
    this.inFlight += byteLength;
    this.totalReceived += byteLength;
    return this.isBackpressured();
  }

  /**
   * Records bytes finished by xterm.js. Returns the byte count to send in a `0x02` ACK
   * frame when the batch threshold is reached, or `0` if more bytes should be coalesced.
   */
  onCommit(byteLength: number, forceFlush = false): number {
    if (byteLength > 0) {
      this.inFlight = Math.max(0, this.inFlight - byteLength);
      this.pendingAck += byteLength;
    }
    if (this.pendingAck >= this.batchBytes || (forceFlush && this.pendingAck > 0)) {
      const toAck = this.pendingAck;
      this.pendingAck = 0;
      this.totalAcked += toAck;
      return toAck;
    }
    return 0;
  }

  isBackpressured(): boolean {
    return this.inFlight >= this.highWatermarkBytes;
  }

  stats(): PtyFlowStats {
    return {
      inFlightBytes: this.inFlight,
      totalReceivedBytes: this.totalReceived,
      totalAckedBytes: this.totalAcked,
      backpressured: this.isBackpressured(),
    };
  }
}
