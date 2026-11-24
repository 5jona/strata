import { useHaptics, useSideVeilPlugin } from '@gojona/sideveil-sdk';
import { type MutableRefObject, useCallback, useState } from 'react';
import type { ServiceControlAction } from '../components/ServicesPanel.tsx';
import { invokeStrataRpc, useStrataEvent } from '../lib/strataRpc.ts';
import {
  type ContainerServiceRuntime,
  STRATA_EVENTS,
  STRATA_METHODS,
  type WebStatusResult,
} from '../types.ts';
import type { StrataNotifyFn } from './useStrataContainers.ts';

export interface UseStrataServicesAndWebOptions {
  isOnline: boolean;
  selectedIdRef: MutableRefObject<string>;
  setBusyAction: (action: string | null) => void;
  notify: StrataNotifyFn;
}

export interface StrataServicesAndWebApi {
  services: ContainerServiceRuntime[];
  serviceLogs: Record<string, string[]>;
  webStatus: WebStatusResult | null;
  pairingToken: string | null;
  refreshServicesAndWeb: (containerId?: string) => Promise<void>;
  controlService: (serviceId: string, action: ServiceControlAction) => Promise<void>;
  loadServiceLogs: (serviceId: string) => Promise<void>;
  toggleWebLan: (lanEnabled: boolean) => Promise<void>;
  rotateWebToken: () => Promise<void>;
}

/**
 * Coordinates managed background services (`service.*`) and PC Desktop pairing (`web.*`)
 * for the narrow SideVeil sidebar plugin.
 */
export function useStrataServicesAndWeb({
  isOnline,
  selectedIdRef,
  setBusyAction,
  notify,
}: UseStrataServicesAndWebOptions): StrataServicesAndWebApi {
  const { bridge } = useSideVeilPlugin();
  const { trigger: triggerHaptic } = useHaptics();

  const [services, setServices] = useState<ContainerServiceRuntime[]>([]);
  const [serviceLogs, setServiceLogs] = useState<Record<string, string[]>>({});
  const [webStatus, setWebStatus] = useState<WebStatusResult | null>(null);
  const [pairingToken, setPairingToken] = useState<string | null>(null);

  const refreshServicesAndWeb = useCallback(
    async (containerId?: string) => {
      if (!isOnline) return;
      const target = containerId ?? selectedIdRef.current;
      const [svcRes, webRes] = await Promise.allSettled([
        invokeStrataRpc(bridge, STRATA_METHODS.SERVICE_LIST, { container: target }),
        invokeStrataRpc(bridge, STRATA_METHODS.WEB_STATUS),
      ]);
      if (svcRes.status === 'fulfilled') setServices(svcRes.value);
      if (webRes.status === 'fulfilled') setWebStatus(webRes.value);
    },
    [bridge, isOnline, selectedIdRef],
  );

  useStrataEvent(STRATA_EVENTS.SERVICE_STATE_CHANGED, (payload) => {
    if (!payload || payload.container !== selectedIdRef.current) return;
    setServices((prev) =>
      prev.map((svc) =>
        svc.id === payload.serviceId
          ? {
              ...svc,
              status: payload.status,
              restartCount: payload.restartCount,
              ...(payload.pid !== undefined ? { pid: payload.pid } : { pid: undefined }),
              ...(payload.lastError !== undefined ? { lastError: payload.lastError } : {}),
            }
          : svc,
      ),
    );
  });

  const controlService = useCallback(
    async (serviceId: string, action: ServiceControlAction) => {
      const target = selectedIdRef.current;
      setBusyAction(`service:${serviceId}`);
      void triggerHaptic('click');
      try {
        const method =
          action === 'start'
            ? STRATA_METHODS.SERVICE_START
            : action === 'stop'
              ? STRATA_METHODS.SERVICE_STOP
              : STRATA_METHODS.SERVICE_RESTART;
        const next = await invokeStrataRpc(bridge, method, { container: target, serviceId });
        setServices((prev) => prev.map((s) => (s.id === serviceId ? next : s)));
        notify(
          action === 'start' ? '服务已启动' : action === 'stop' ? '服务已停止' : '服务已重启',
          'success',
          next.name,
        );
      } catch (err) {
        notify('服务操作失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, selectedIdRef, setBusyAction, triggerHaptic],
  );

  const loadServiceLogs = useCallback(
    async (serviceId: string) => {
      const target = selectedIdRef.current;
      try {
        const res = await invokeStrataRpc(bridge, STRATA_METHODS.SERVICE_LOGS, {
          container: target,
          serviceId,
          limit: 80,
        });
        setServiceLogs((prev) => ({ ...prev, [serviceId]: res.lines }));
      } catch (err) {
        notify('服务日志读取失败', 'danger', err instanceof Error ? err.message : String(err));
      }
    },
    [bridge, notify, selectedIdRef],
  );

  const toggleWebLan = useCallback(
    async (lanEnabled: boolean) => {
      setBusyAction('web:lan');
      void triggerHaptic('tick');
      try {
        const next = await invokeStrataRpc(bridge, STRATA_METHODS.WEB_CONFIGURE, { lanEnabled });
        setWebStatus(next);
        notify(lanEnabled ? '已开启局域网配对' : '已切换为本机回环', 'success');
      } catch (err) {
        notify('配对配置更新失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, setBusyAction, triggerHaptic],
  );

  const rotateWebToken = useCallback(async () => {
    setBusyAction('web:token');
    void triggerHaptic('click');
    try {
      const res = await invokeStrataRpc(bridge, STRATA_METHODS.WEB_ROTATE_TOKEN);
      setPairingToken(res.token);
      notify('Bearer Token 已轮换', 'success', `已吊销 ${res.sessionsRevoked} 个旧会话`);
    } catch (err) {
      notify('Token 轮换失败', 'danger', err instanceof Error ? err.message : String(err));
    } finally {
      setBusyAction(null);
    }
  }, [bridge, notify, setBusyAction, triggerHaptic]);

  return {
    services,
    serviceLogs,
    webStatus,
    pairingToken,
    refreshServicesAndWeb,
    controlService,
    loadServiceLogs,
    toggleWebLan,
    rotateWebToken,
  };
}
