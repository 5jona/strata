import { type BridgeOptions, BridgeProvider } from '@gojona/bridge';
import {
  AppShell,
  Badge,
  Button,
  Card,
  ErrorBoundary,
  StatusBanner,
  ThemeProvider,
  ToastProvider,
} from '@gojona/ui';
import { Layers, Play, RefreshCw } from 'lucide-react';
import { useMemo } from 'react';
import { KsuTabsWorkspace } from './KsuTabsWorkspace.tsx';
import { createMockKsuStrataTransport } from './ksuStrataClient.ts';
import { useKsuStrataRuntime } from './useKsuStrataRuntime.ts';

function KsuStrataWorkbench() {
  const rt = useKsuStrataRuntime();
  const active = rt.activeContainer;
  const daemonOnline = rt.daemon !== null && !rt.lastError;

  return (
    <AppShell
      variant="page"
      maxWidthClassName="max-w-none"
      title="Strata 容器"
      subtitle={
        rt.daemon
          ? `本地容器管理与应急恢复 · 运行 ${rt.daemon.runningContainers}/${rt.daemon.totalContainers}`
          : '本地容器管理与应急恢复'
      }
      bodyClassName="flex flex-col px-3 py-3 sm:px-5"
      headerLeading={
        <div className="flex size-9 items-center justify-center rounded-[var(--radius-control)] bg-[var(--accent-soft)] text-[var(--accent)]">
          <Layers className="size-4" />
        </div>
      }
      headerTrailing={
        <div className="flex items-center gap-2">
          <Badge variant={daemonOnline ? 'success' : 'danger'} dot pulse={daemonOnline}>
            {daemonOnline ? '服务在线' : '服务离线'}
          </Badge>
          <Button
            size="sm"
            variant="secondary"
            aria-label="刷新状态"
            disabled={rt.loading}
            onClick={() => void rt.refreshAll()}
          >
            <RefreshCw className={`size-3.5 ${rt.loading ? 'animate-spin' : ''}`} />
            刷新
          </Button>
        </div>
      }
    >
      <div className="flex w-full grow shrink-0 flex-col gap-3">
        {!rt.loading && !daemonOnline ? (
          <StatusBanner
            tone="danger"
            title="Strata 后台服务离线或暂不可达"
            description={rt.lastError ?? '无法连接后台服务，请点击右侧按钮尝试启动后台服务。'}
            action={
              <Button
                size="sm"
                variant="primary"
                disabled={rt.busyKey === 'daemon:rescue'}
                onClick={() => void rt.startDaemon()}
              >
                <Play className="size-3.5" />
                启动后台服务
              </Button>
            }
          />
        ) : null}

        {rt.containers.length > 1 ? (
          <div className="flex shrink-0 flex-wrap gap-1.5">
            {rt.containers.map((c) => (
              <Button
                key={c.id}
                size="sm"
                variant={c.id === rt.selectedId ? 'pill' : 'secondary'}
                onClick={() => rt.selectContainer(c.id)}
              >
                {c.id}
              </Button>
            ))}
          </div>
        ) : null}

        {rt.loading && !active ? (
          <Card className="p-4 text-center text-xs text-[var(--text-secondary)]">
            正在读取 Strata 容器状态…
          </Card>
        ) : (
          <KsuTabsWorkspace active={active} daemonOnline={daemonOnline} rt={rt} />
        )}
      </div>
    </AppShell>
  );
}

/** Root standalone KernelSU WebUI component (`strata/webroot/`). */
export function KsuStrataApp() {
  const bridgeOptions = useMemo<BridgeOptions>(
    () => ({
      mockNamespace: 'strata-ksu-webui',
      mockTransport: createMockKsuStrataTransport(),
    }),
    [],
  );

  return (
    <ErrorBoundary>
      <BridgeProvider options={bridgeOptions}>
        <ThemeProvider>
          <ToastProvider>
            <KsuStrataWorkbench />
          </ToastProvider>
        </ThemeProvider>
      </BridgeProvider>
    </ErrorBoundary>
  );
}
