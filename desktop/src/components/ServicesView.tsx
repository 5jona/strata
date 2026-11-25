import { Button, EmptyState, LogConsole, type LogEntry, useToast } from '@gojona/ui';
import { Plus, RefreshCw, ServerCog } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type {
  ContainerServiceRuntime,
  ContainerServiceSpec,
  ContainerStatusSnapshot,
} from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';
import { ServiceModal } from './ServiceModal.tsx';
import { ServiceRowCard } from './ServiceRowCard.tsx';

export interface ServicesViewProps {
  transport: StrataTransport | null;
  containers: ContainerStatusSnapshot[];
  servicesByContainer: Record<string, ContainerServiceRuntime[]>;
  onRefreshServices: (containerId: string) => Promise<void>;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * First-class Managed Multi-Service Auto-Start control view with inline log viewer.
 */
export function ServicesView({
  transport,
  containers,
  servicesByContainer,
  onRefreshServices,
  runAction,
}: ServicesViewProps) {
  const { pushToast } = useToast();
  const [selectedContainer, setSelectedContainer] = useState<string>(containers[0]?.id ?? 'main');
  const [specs, setSpecs] = useState<ContainerServiceSpec[]>([]);
  const [modalOpen, setModalOpen] = useState(false);
  const [editingSpec, setEditingSpec] = useState<ContainerServiceSpec | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [logServiceId, setLogServiceId] = useState<string | null>(null);
  const [logLines, setLogLines] = useState<string[]>([]);

  useEffect(() => {
    if (containers.length > 0 && !containers.some((c) => c.id === selectedContainer)) {
      setSelectedContainer(containers[0].id);
    }
  }, [containers, selectedContainer]);

  const loadSpecs = useCallback(async () => {
    if (!transport || !selectedContainer) return;
    try {
      const cfg = await transport.invoke(STRATA_METHODS.CONTAINER_CONFIG_GET, {
        container: selectedContainer,
      });
      setSpecs(cfg.config.services ?? []);
    } catch {
      setSpecs([]);
    }
  }, [selectedContainer, transport]);

  useEffect(() => {
    void loadSpecs();
  }, [loadSpecs]);

  const loadServiceLogs = useCallback(
    async (serviceId: string) => {
      if (!transport) return;
      setLogServiceId(serviceId);
      const res = await runAction('拉取服务日志', () =>
        transport.invoke(STRATA_METHODS.SERVICE_LOGS, {
          container: selectedContainer,
          serviceId,
          limit: 200,
        }),
      );
      if (res) setLogLines(res.lines);
    },
    [runAction, selectedContainer, transport],
  );

  const services = servicesByContainer[selectedContainer] ?? [];
  const logEntries: LogEntry[] = logLines.map((line, idx) => ({
    id: `${logServiceId ?? 'svc'}-${idx}`,
    timestamp: String(idx + 1).padStart(3, '0'),
    level: /error|fail|panic/i.test(line) ? 'error' : /warn/i.test(line) ? 'warn' : 'info',
    message: line,
  }));

  const runServiceAction = async (serviceId: string, label: string, fn: () => Promise<unknown>) => {
    if (!transport) return;
    setBusyId(serviceId);
    try {
      await runAction(label, fn);
      await onRefreshServices(selectedContainer);
    } finally {
      setBusyId(null);
    }
  };

  const handleToggleAutoStart = async (svc: ContainerServiceRuntime, next: boolean) => {
    if (!transport) return;
    const spec = specs.find((s) => s.id === svc.id);
    if (!spec) return;
    await runServiceAction(svc.id, '更新开机自启', async () => {
      await transport.invoke(STRATA_METHODS.SERVICE_UPSERT, {
        container: selectedContainer,
        spec: { ...spec, autoStart: next },
      });
      await loadSpecs();
      pushToast(`服务 ${svc.name} 已${next ? '开启' : '关闭'}自启`, 'success');
    });
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">
            托管后台服务 (开机静默自启)
          </h1>
          <p className="text-xs text-[var(--text-secondary)]">
            无需 systemd 即可在容器内守护常驻服务，支持崩溃退避重启与日志追踪
          </p>
        </div>
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-1">
            {containers.map((c) => (
              <Button
                key={c.id}
                size="sm"
                variant={selectedContainer === c.id ? 'pill' : 'ghost'}
                onClick={() => {
                  setSelectedContainer(c.id);
                  setLogServiceId(null);
                }}
              >
                {c.id}
              </Button>
            ))}
          </div>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => void Promise.all([onRefreshServices(selectedContainer), loadSpecs()])}
          >
            <RefreshCw className="size-3.5" />
            刷新
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              setEditingSpec(null);
              setModalOpen(true);
            }}
          >
            <Plus className="size-3.5" />
            添加服务
          </Button>
        </div>
      </div>

      {services.length === 0 ? (
        <EmptyState
          icon={<ServerCog className="size-8" />}
          title={`容器 ${selectedContainer} 尚未配置托管服务`}
          description="点击「添加服务」注册 RSSHub、Syncthing、Caddy 或自定义常驻脚本。"
          action={
            <Button
              size="sm"
              variant="primary"
              onClick={() => {
                setEditingSpec(null);
                setModalOpen(true);
              }}
            >
              添加第一个服务
            </Button>
          }
        />
      ) : (
        <div className="flex flex-col gap-2.5">
          {services.map((svc) => {
            const spec = specs.find((s) => s.id === svc.id);
            return (
              <ServiceRowCard
                key={svc.id}
                service={svc}
                spec={spec}
                busy={busyId === svc.id}
                logActive={logServiceId === svc.id}
                onToggleAutoStart={(v) => void handleToggleAutoStart(svc, v)}
                onStart={() =>
                  void runServiceAction(
                    svc.id,
                    `启动 ${svc.name}`,
                    () =>
                      transport?.invoke(STRATA_METHODS.SERVICE_START, {
                        container: selectedContainer,
                        serviceId: svc.id,
                      }) ?? Promise.resolve(),
                  )
                }
                onStop={() =>
                  void runServiceAction(
                    svc.id,
                    `停止 ${svc.name}`,
                    () =>
                      transport?.invoke(STRATA_METHODS.SERVICE_STOP, {
                        container: selectedContainer,
                        serviceId: svc.id,
                      }) ?? Promise.resolve(),
                  )
                }
                onRestart={() =>
                  void runServiceAction(
                    svc.id,
                    `重启 ${svc.name}`,
                    () =>
                      transport?.invoke(STRATA_METHODS.SERVICE_RESTART, {
                        container: selectedContainer,
                        serviceId: svc.id,
                      }) ?? Promise.resolve(),
                  )
                }
                onViewLogs={() => void loadServiceLogs(svc.id)}
                onEdit={() => {
                  setEditingSpec(
                    spec ?? {
                      id: svc.id,
                      name: svc.name,
                      command: [],
                      env: {},
                      autoStart: svc.autoStart,
                      restartPolicy: 'on-failure',
                      maxRestarts: 5,
                      backoffMs: 2000,
                    },
                  );
                  setModalOpen(true);
                }}
                onDelete={() =>
                  void runServiceAction(svc.id, `删除 ${svc.name}`, async () => {
                    await transport?.invoke(STRATA_METHODS.SERVICE_DELETE, {
                      container: selectedContainer,
                      serviceId: svc.id,
                    });
                    await loadSpecs();
                    if (logServiceId === svc.id) setLogServiceId(null);
                  })
                }
              />
            );
          })}
        </div>
      )}

      {logServiceId && (
        <LogConsole
          title={`服务日志 · ${selectedContainer}/${logServiceId}`}
          entries={logEntries}
          searchable
          filterable
          maxHeightClassName="max-h-72"
          emptyMessage="暂无日志输出"
        />
      )}

      <ServiceModal
        open={modalOpen}
        containerId={selectedContainer}
        initialSpec={editingSpec}
        busy={busyId !== null}
        onClose={() => setModalOpen(false)}
        onSubmit={async (spec) => {
          if (!transport) return;
          await runServiceAction(spec.id, '保存托管服务', async () => {
            await transport.invoke(STRATA_METHODS.SERVICE_UPSERT, {
              container: selectedContainer,
              spec,
            });
            await loadSpecs();
            pushToast(`已保存服务 ${spec.name}`, 'success');
          });
        }}
      />
    </div>
  );
}
