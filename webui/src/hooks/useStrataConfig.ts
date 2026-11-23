import { useHaptics, useSideVeilPlugin } from '@gojona/sideveil-sdk';
import type { LogEntry } from '@gojona/ui';
import { type RefObject, useCallback, useState } from 'react';
import { formatClockTime } from '../lib/strataFormat.ts';
import { invokeStrataRpc } from '../lib/strataRpc.ts';
import { type ContainerConfig, type ContainerConfigPatchParams, STRATA_METHODS } from '../types.ts';
import type { StrataNotifyFn } from './useStrataContainers.ts';

/** Input dependencies for `useStrataConfig`. */
export interface UseStrataConfigOptions {
  isOnline: boolean;
  selectedIdRef: RefObject<string>;
  setBusyAction: (action: string | null) => void;
  notify: StrataNotifyFn;
  refreshAll: () => Promise<void>;
}

/** Per-container configuration, SSH keys, and Dropbear log controller. */
export interface StrataConfigApi {
  dropbearLogs: LogEntry[];
  containerConfig: ContainerConfig | null;
  sshKeys: string[];
  refreshConfigAndLogs: (containerId?: string) => Promise<void>;
  patchConfig: (patch: ContainerConfigPatchParams) => Promise<boolean>;
  addSshKey: (publicKey: string) => Promise<boolean>;
}

/**
 * Loads and mutates per-container configuration, SSH authorized keys, and Dropbear logs.
 */
export function useStrataConfig({
  isOnline,
  selectedIdRef,
  setBusyAction,
  notify,
  refreshAll,
}: UseStrataConfigOptions): StrataConfigApi {
  const { bridge } = useSideVeilPlugin();
  const { trigger: triggerHaptic } = useHaptics();

  const [dropbearLogs, setDropbearLogs] = useState<LogEntry[]>([]);
  const [containerConfig, setContainerConfig] = useState<ContainerConfig | null>(null);
  const [sshKeys, setSshKeys] = useState<string[]>([]);

  const refreshConfigAndLogs = useCallback(
    async (targetId?: string) => {
      if (!isOnline) return;
      const cid = targetId ?? selectedIdRef.current;
      try {
        const [logsRes, cfgRes, keysRes] = await Promise.all([
          invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_LOGS, { container: cid, lines: 80 }),
          invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_CONFIG_GET, { container: cid }),
          invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_SSH_KEY_LIST, { container: cid }),
        ]);
        if (selectedIdRef.current !== cid) return;
        const ts = formatClockTime(Date.now());
        setDropbearLogs(
          (logsRes.lines ?? []).map((line, index) => ({
            id: `db-${index}-${line.slice(0, 12)}`,
            timestamp: ts,
            level: /error|fail|denied/i.test(line) ? 'warn' : 'info',
            message: line,
          })),
        );
        setContainerConfig(cfgRes.config);
        setSshKeys(keysRes.keys ?? []);
      } catch {
        // Secondary config/log data may fail transiently while container stops/starts.
      }
    },
    [bridge, isOnline, selectedIdRef],
  );

  const patchConfig = useCallback(
    async (patch: ContainerConfigPatchParams): Promise<boolean> => {
      const cid = selectedIdRef.current;
      setBusyAction('config');
      void triggerHaptic('click');
      try {
        const res = await invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_CONFIG_PATCH, {
          ...patch,
          container: cid,
        });
        setContainerConfig(res.config);
        notify('配置已保存', 'success', '重启动容器后生效');
        await refreshAll();
        return true;
      } catch (err) {
        notify('保存配置失败', 'danger', err instanceof Error ? err.message : String(err));
        return false;
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, refreshAll, selectedIdRef, setBusyAction, triggerHaptic],
  );

  const addSshKey = useCallback(
    async (publicKey: string): Promise<boolean> => {
      const cid = selectedIdRef.current;
      setBusyAction('ssh-key');
      void triggerHaptic('click');
      try {
        const res = await invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_SSH_KEY_ADD, {
          container: cid,
          publicKey,
        });
        setSshKeys(res.keys);
        notify(res.added > 0 ? '公钥已添加' : '公钥已存在', 'success', `共 ${res.keys.length} 条`);
        return true;
      } catch (err) {
        notify('添加公钥失败', 'danger', err instanceof Error ? err.message : String(err));
        return false;
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, selectedIdRef, setBusyAction, triggerHaptic],
  );

  return {
    dropbearLogs,
    containerConfig,
    sshKeys,
    refreshConfigAndLogs,
    patchConfig,
    addSshKey,
  };
}
