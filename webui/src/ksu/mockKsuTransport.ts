import { MockTransport } from '@gojona/bridge';
import { handleMockPluginInvoke } from '../shared/mock/handlers.ts';
import { createInitialMockState } from '../shared/mock/state.ts';

function unquoteSingle(token: string): string {
  if (token.startsWith("'") && token.endsWith("'")) {
    return token.slice(1, -1).replaceAll("'\\''", "'");
  }
  return token;
}

/**
 * Creates a standalone preview `MockTransport` handling `strata rpc`, `strata --json web token`,
 * and `service.sh --now` when opening the KSU WebUI in a desktop browser outside KernelSU Manager.
 */
export function createMockKsuStrataTransport(): MockTransport {
  const state = createInitialMockState();
  return new MockTransport({
    namespace: 'strata-ksu-preview',
    onExec: (cmd) => {
      if (cmd.includes('service.sh') && cmd.includes('--now')) {
        return { errno: 0, stdout: '', stderr: '' };
      }
      if (cmd.includes('web token')) {
        return {
          errno: 0,
          stdout: JSON.stringify({ token: state.webToken, tokenPath: state.webStatus.tokenPath }),
          stderr: '',
        };
      }
      const normalizedCmd = cmd.replace(/\s+<\/dev\/null\s*$/, '');
      const rpcMatch = normalizedCmd.match(/\brpc\s+('[^']*'|\S+)(?:\s+(.+))?$/);
      if (!rpcMatch) {
        return { errno: 1, stdout: '', stderr: `unsupported mock command: ${cmd}` };
      }
      const method = unquoteSingle(rpcMatch[1] ?? '');
      const rawJson = rpcMatch[2] ? unquoteSingle(rpcMatch[2].trim()) : undefined;
      const params = rawJson ? (JSON.parse(rawJson) as unknown) : undefined;
      const result = handleMockPluginInvoke(state, () => {}, method, params);
      return { errno: 0, stdout: JSON.stringify(result), stderr: '' };
    },
  });
}
