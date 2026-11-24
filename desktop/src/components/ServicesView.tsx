import {
  Badge,
  Button,
  Card,
  EmptyState,
  LogConsole,
  type LogEntry,
  Switch,
  useToast,
} from '@gojona/ui';
import {
  FileText,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  ServerCog,
  Square,
  Trash2,
} from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { formatUptime, serviceStatusMeta } from '../lib/format.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type {
  ContainerServiceRuntime,
  ContainerServiceSpec,
  ContainerStatusSnapshot,
} from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';
import { ServiceModal } from './ServiceModal.tsx';

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

  const handleToggleAutoStart = async (svc: ContainerServiceRuntime, next: boolean) => {
    if (!transport) return;
    const spec = specs.find((s) => s.id === svc.id);
    if (!spec) return;
    setBusyId(svc.id);
    try {
      await runAction('更新开机自启', () =>
        transport.invoke(STRATA_METHODS.SERVICE_UPSERT, {
          container: selectedContainer,
          spec: { ...spec, autoStart: next },
        }),
      );
      await Promise.all([onRefreshServices(selectedContainer), loadSpecs()]);
      pushToast(`服务 ${svc.name} 已${next ? '开启' : '关闭'}自启`, 'success');
    } finally {
      setBusyId(null);
    }
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
            const meta = serviceStatusMeta(svc.status);
            const spec = specs.find((s) => s.id === svc.id);
            const isRunning = svc.status === 'running';
            return (
              <Card
                key={svc.id}
                className="flex flex-wrap items-center justify-between gap-3 p-3.5"
              >
                <div className="min-w-[220px] flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-semibold text-[var(--text-primary)]">
                      {svc.name}
                    </span>
                    <span className="font-mono text-xs text-[var(--text-secondary)]">
                      ({svc.id})
                    </span>
                    <Badge variant={meta.variant}>{meta.label}</Badge>
                    {svc.pid && <Badge variant="neutral">PID {svc.pid}</Badge>}
                  </div>
                  <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
                    {spec && <span className="truncate">{spec.command.join(' ')}</span>}
                    <span>运行 {formatUptime(svc.uptimeMs)}</span>
                    <span>重启 {svc.restartCount} 次</span>
                    {svc.lastError && (
                      <span className="text-[var(--danger)]">错误：{svc.lastError}</span>
                    )}
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-2">
                  <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
                    <span>开机自启</span>
                    <Switch
                      checked={svc.autoStart}
                      disabled={busyId === svc.id}
                      aria-label={`开机自启 ${svc.name}`}
                      onCheckedChange={(v) => void handleToggleAutoStart(svc, v)}
                    />
                  </div>
                  {isRunning ? (
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={busyId === svc.id}
                      onClick={async () => {
                        if (!transport) return;
                        setBusyId(svc.id);
                        await runAction(`停止 ${svc.name}`, () =>
                          transport.invoke(STRATA_METHODS.SERVICE_STOP, {
                            container: selectedContainer,
                            serviceId: svc.id,
                          }),
                        );
                        await onRefreshServices(selectedContainer);
                        setBusyId(null);
                      }}
                    >
                      <Square className="size-3.5" />
                      停止
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      variant="primary"
                      disabled={busyId === svc.id}
                      onClick={async () => {
                        if (!transport) return;
                        setBusyId(svc.id);
                        await runAction(`启动 ${svc.name}`, () =>
                          transport.invoke(STRATA_METHODS.SERVICE_START, {
                            container: selectedContainer,
                            serviceId: svc.id,
                          }),
                        );
                        await onRefreshServices(selectedContainer);
                        setBusyId(null);
                      }}
                    >
                      <Play className="size-3.5" />
                      启动
                    </Button>
                  )}
                  <Button
                    size="icon-sm"
                    variant="secondary"
                    title="重启服务"
                    aria-label={`重启 ${svc.name}`}
                    disabled={busyId === svc.id}
                    onClick={async () => {
                      if (!transport) return;
                      setBusyId(svc.id);
                      await runAction(`重启 ${svc.name}`, () =>
                        transport.invoke(STRATA_METHODS.SERVICE_RESTART, {
                          container: selectedContainer,
                          serviceId: svc.id,
                        }),
                      );
                      await onRefreshServices(selectedContainer);
                      setBusyId(null);
                    }}
                  >
                    <RotateCw className="size-3.5" />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant={logServiceId === svc.id ? 'pill' : 'secondary'}
                    title="查看日志"
                    aria-label={`查看 ${svc.name} 日志`}
                    onClick={() => void loadServiceLogs(svc.id)}
                  >
                    <FileText className="size-3.5" />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    title="编辑服务"
                    aria-label={`编辑 ${svc.name}`}
                    onClick={() => {
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
                  >
                    <Pencil className="size-3.5" />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    title="删除服务"
                    aria-label={`删除 ${svc.name}`}
                    onClick={async () => {
                      if (!transport) return;
                      setBusyId(svc.id);
                      await runAction(`删除 ${svc.name}`, () =>
                        transport.invoke(STRATA_METHODS.SERVICE_DELETE, {
                          container: selectedContainer,
                          serviceId: svc.id,
                        }),
                      );
                      await Promise.all([onRefreshServices(selectedContainer), loadSpecs()]);
                      if (logServiceId === svc.id) setLogServiceId(null);
                      setBusyId(null);
                    }}
                  >
                    <Trash2 className="size-3.5 text-[var(--danger)]" />
                  </Button>
                </div>
              </Card>
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
          setBusyId(spec.id);
          try {
            await runAction('保存托管服务', () =>
              transport.invoke(STRATA_METHODS.SERVICE_UPSERT, {
                container: selectedContainer,
                spec,
              }),
            );
            await Promise.all([onRefreshServices(selectedContainer), loadSpecs()]);
            pushToast(`已保存服务 ${spec.name}`, 'success');
          } finally {
            setBusyId(null);
          }
        }}
      />
    </div>
  );
}
