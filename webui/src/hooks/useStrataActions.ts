import { useHaptics, useSideVeilPlugin } from '@gojona/sideveil-sdk';
import { type RefObject, useCallback } from 'react';
import { formatBytes } from '../lib/strataFormat.ts';
import { invokeStrataRpc } from '../lib/strataRpc.ts';
import { STRATA_METHODS } from '../types.ts';
import type { StrataNotifyFn } from './useStrataContainers.ts';

/** Supported container lifecycle commands. */
export type ContainerLifecycleAction = 'start' | 'stop' | 'restart';

/** Supported container maintenance operations. */
export type ContainerMaintenanceOp = 'dns' | 'compact' | 'fsck' | 'backup';

/** Input dependencies for `useStrataActions`. */
export interface UseStrataActionsOptions {
  selectedIdRef: RefObject<string>;
  setBusyAction: (action: string | null) => void;
  notify: StrataNotifyFn;
  refreshAll: () => Promise<void>;
}

/** Lifecycle and maintenance action handlers exposed by `useStrataActions`. */
export interface StrataActionsApi {
  runLifecycle: (action: ContainerLifecycleAction) => Promise<void>;
  runMaintenance: (op: ContainerMaintenanceOp) => Promise<void>;
}

/**
 * Provides container lifecycle (`start`/`stop`/`restart`) and storage/network maintenance RPC actions.
 */
export function useStrataActions({
  selectedIdRef,
  setBusyAction,
  notify,
  refreshAll,
}: UseStrataActionsOptions): StrataActionsApi {
  const { bridge } = useSideVeilPlugin();
  const { trigger: triggerHaptic } = useHaptics();

  const runLifecycle = useCallback(
    async (action: ContainerLifecycleAction) => {
      const cid = selectedIdRef.current;
      setBusyAction(action);
      void triggerHaptic(action === 'stop' ? 'heavy' : 'click');
      try {
        if (action === 'start') {
          await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_START,
            { container: cid },
            { timeoutMs: 20_000 },
          );
          notify('容器已启动', 'success', cid);
        } else if (action === 'stop') {
          const report = await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_STOP,
            { container: cid },
            { timeoutMs: 20_000 },
          );
          notify(
            '容器已停止',
            'success',
            `清理挂载=${report.unmountedPaths} 残留=${report.residualMounts}/${report.residualLoops}`,
          );
        } else {
          await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_RESTART,
            { container: cid },
            { timeoutMs: 25_000 },
          );
          notify('容器已重启', 'success', cid);
        }
        await refreshAll();
      } catch (err) {
        notify('操作失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, refreshAll, selectedIdRef, setBusyAction, triggerHaptic],
  );

  const runMaintenance = useCallback(
    async (op: ContainerMaintenanceOp) => {
      const cid = selectedIdRef.current;
      setBusyAction(op);
      void triggerHaptic('click');
      try {
        if (op === 'dns') {
          await invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_SYNC_DNS, { container: cid });
          notify('DNS 与宿主机桥接已同步', 'success', cid);
        } else if (op === 'compact') {
          const res = await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_COMPACT,
            { container: cid },
            { timeoutMs: 45_000 },
          );
          notify(
            'FITRIM 空间回收完成',
            'success',
            `${formatBytes(res.beforeBytes)} → ${formatBytes(res.afterBytes)}`,
          );
        } else if (op === 'fsck') {
          const res = await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_FSCK,
            { container: cid },
            { timeoutMs: 45_000 },
          );
          notify('磁盘检查完成', res.exitCode <= 1 ? 'success' : 'danger', `exit=${res.exitCode}`);
        } else {
          const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
          const outputPath = `/data/local/tmp/strata-${cid}-${stamp}.img.gz`;
          const res = await invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_BACKUP_START, {
            container: cid,
            outputPath,
          });
          notify('后台压缩备份已启动', 'info', res.outputPath);
        }
        await refreshAll();
      } catch (err) {
        notify('运维任务失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, refreshAll, selectedIdRef, setBusyAction, triggerHaptic],
  );

  return { runLifecycle, runMaintenance };
}
