import assert from 'node:assert/strict';
import test from 'node:test';
import {
  decodeServerPtyFrame,
  encodePtyAckFrame,
  encodePtyDataFrame,
  encodePtyResizeFrame,
  PTY_ACK_BATCH_BYTES,
  PTY_FRAME_ACK,
  PTY_FRAME_DATA,
  PTY_FRAME_EXIT,
  PTY_FRAME_RESIZE,
  PTY_HIGH_WATERMARK_BYTES,
  PtyWatermarkController,
  parsePtySessionMeta,
} from '../src/transport/ptyProtocol.ts';

test('encodes binary PTY data, resize, and u32 BE ACK frames', () => {
  const data = encodePtyDataFrame('ls -la\n');
  assert.equal(data[0], PTY_FRAME_DATA);
  assert.equal(new TextDecoder().decode(data.subarray(1)), 'ls -la\n');

  const resize = encodePtyResizeFrame(120, 40);
  assert.equal(resize[0], PTY_FRAME_RESIZE);
  assert.deepEqual(JSON.parse(new TextDecoder().decode(resize.subarray(1))), {
    cols: 120,
    rows: 40,
  });

  const ack = encodePtyAckFrame(65536);
  assert.equal(ack.byteLength, 5);
  assert.equal(ack[0], PTY_FRAME_ACK);
  assert.equal(new DataView(ack.buffer).getUint32(1, false), 65536);
});

test('decodes server PTY data, exit, and handshake metadata frames', () => {
  const rawData = encodePtyDataFrame('hello strata');
  const decodedData = decodeServerPtyFrame(rawData);
  assert.equal(decodedData.type, 'data');
  assert.equal(new TextDecoder().decode(decodedData.payload), 'hello strata');

  const exitJson = new TextEncoder().encode(JSON.stringify({ exitCode: 130, signal: 2 }));
  const exitBuf = new Uint8Array(1 + exitJson.length);
  exitBuf[0] = PTY_FRAME_EXIT;
  exitBuf.set(exitJson, 1);
  const decodedExit = decodeServerPtyFrame(exitBuf);
  assert.deepEqual(decodedExit, { type: 'exit', exitCode: 130, signal: 2 });

  const meta = parsePtySessionMeta('{"sessionId":"s-1","container":"main","pid":42}');
  assert.deepEqual(meta, { sessionId: 's-1', container: 'main', pid: 42 });
  assert.equal(parsePtySessionMeta('not-json'), null);
});

test('PtyWatermarkController batches 64 KiB ACKs and tracks 256 KiB high watermark', () => {
  const ctrl = new PtyWatermarkController(PTY_ACK_BATCH_BYTES, PTY_HIGH_WATERMARK_BYTES);
  assert.equal(ctrl.onReceive(128 * 1024), false);
  assert.equal(ctrl.onReceive(128 * 1024), true);
  assert.equal(ctrl.stats().backpressured, true);

  // Committing 32 KiB does not yet cross 64 KiB ACK batch threshold
  assert.equal(ctrl.onCommit(32 * 1024), 0);
  // Committing another 32 KiB flushes a single 64 KiB ACK
  assert.equal(ctrl.onCommit(32 * 1024), 64 * 1024);
  assert.equal(ctrl.stats().backpressured, false);
  assert.equal(ctrl.stats().totalAckedBytes, 64 * 1024);
});
