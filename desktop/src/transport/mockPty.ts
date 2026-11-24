import type { PtyConnectionHandlers, PtySessionHandle } from './httpWsTransport.ts';

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Interactive mock PTY session for standalone browser preview and automated tests.
 * Supports `help`, `ls`, `uname -a`, `uptime`, `whoami`, `pwd`, `clear`, `yes` (flow-control demo), and Ctrl-C.
 */
export function createMockPtySession(
  containerId: string,
  hostname: string,
  handlers: PtyConnectionHandlers,
  resumeSessionId?: string,
): PtySessionHandle {
  const sessionId = resumeSessionId ?? `mock-pty-${containerId}-${Date.now().toString(36)}`;
  let closed = false;
  let lineBuffer = '';
  let burstTimer: ReturnType<typeof setInterval> | null = null;
  let burstCount = 0;

  const emitText = (text: string) => {
    if (!closed) {
      handlers.onData(encoder.encode(text));
    }
  };

  const prompt = () => `\x1b[1;36mroot@${hostname}\x1b[0m:\x1b[1;34m~\x1b[0m# `;

  const stopBurst = () => {
    if (burstTimer) {
      clearInterval(burstTimer);
      burstTimer = null;
    }
  };

  const runCommand = (raw: string) => {
    const cmd = raw.trim();
    if (!cmd) {
      emitText(prompt());
      return;
    }
    if (cmd === 'help') {
      emitText(
        `Strata 预览终端命令: help, ls, uname -a, uptime, whoami, pwd, yes (流控压测), clear, exit\r\n${prompt()}`,
      );
    } else if (cmd === 'ls') {
      emitText(`bin  dev  etc  home  opt  proc  root  run  sys  tmp  usr  var\r\n${prompt()}`);
    } else if (cmd.startsWith('uname')) {
      emitText(
        `Linux ${hostname} 6.6.56-android16-8-gki #1 SMP PREEMPT aarch64 GNU/Linux\r\n${prompt()}`,
      );
    } else if (cmd === 'uptime') {
      emitText(` 21:48:00 up 4:07,  1 user,  load average: 0.24, 0.19, 0.15\r\n${prompt()}`);
    } else if (cmd === 'whoami') {
      emitText(`root\r\n${prompt()}`);
    } else if (cmd === 'pwd') {
      emitText(`/root\r\n${prompt()}`);
    } else if (cmd === 'clear') {
      emitText(`\x1b[2J\x1b[H${prompt()}`);
    } else if (cmd === 'yes') {
      emitText('启动流控输出压测（按 Ctrl+C 停止）...\r\n');
      burstCount = 0;
      const chunk = `${'strata-flow-control-watermark-ack-test-line-0123456789\r\n'.repeat(64)}`;
      burstTimer = setInterval(() => {
        if (closed || burstCount >= 80) {
          stopBurst();
          if (!closed) emitText(`\r\n[压测完成，共发送 ${burstCount} 批]\r\n${prompt()}`);
          return;
        }
        burstCount += 1;
        emitText(chunk);
      }, 25);
    } else if (cmd === 'exit') {
      closed = true;
      stopBurst();
      handlers.onExit?.(0);
      handlers.onClose?.('shell exited');
    } else {
      emitText(`sh: ${cmd}: not found (输入 help 查看可用预览命令)\r\n${prompt()}`);
    }
  };

  setTimeout(() => {
    if (closed) return;
    handlers.onOpen?.();
    handlers.onMeta?.({ sessionId, container: containerId, pid: 19402 });
    if (resumeSessionId) {
      emitText(`\r\n\x1b[32m[已重连会话 ${sessionId}]\x1b[0m\r\n${prompt()}`);
    } else {
      emitText(
        `\x1b[1;32mStrata PTY 已连接\x1b[0m · 容器 \x1b[1m${containerId}\x1b[0m (输入 \x1b[33mhelp\x1b[0m 或 \x1b[33myes\x1b[0m 测试流控)\r\n${prompt()}`,
      );
    }
  }, 30);

  return {
    sendInput(input) {
      if (closed) return;
      const text = typeof input === 'string' ? input : decoder.decode(input);
      for (const ch of text) {
        if (ch === '\x03') {
          stopBurst();
          lineBuffer = '';
          emitText(`^C\r\n${prompt()}`);
        } else if (ch === '\r' || ch === '\n') {
          emitText('\r\n');
          const cmd = lineBuffer;
          lineBuffer = '';
          runCommand(cmd);
        } else if (ch === '\x7f' || ch === '\b') {
          if (lineBuffer.length > 0) {
            lineBuffer = lineBuffer.slice(0, -1);
            emitText('\b \b');
          }
        } else if (ch >= ' ') {
          lineBuffer += ch;
          emitText(ch);
        }
      }
    },
    sendResize(_cols, _rows) {
      // No-op in mock PTY
    },
    sendAck(_bytes) {
      // Acknowledged by mock transport
    },
    close() {
      closed = true;
      stopBurst();
    },
  };
}
