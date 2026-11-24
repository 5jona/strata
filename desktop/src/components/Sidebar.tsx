import { Badge, Button, cn, ThemeSwitcher } from '@gojona/ui';
import {
  Box,
  Cpu,
  Download,
  Layers,
  Network,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  RefreshCw,
  ServerCog,
  Smartphone,
  Terminal,
  Trash2,
  Wifi,
} from 'lucide-react';
import type { ConnectionPhase } from '../hooks/useDesktopDevices.ts';
import type { DeviceProfile, NavViewId } from '../types/device.ts';

export interface SidebarProps {
  collapsed: boolean;
  onToggleCollapse: () => void;
  activeView: NavViewId;
  onSelectView: (view: NavViewId) => void;
  devices: DeviceProfile[];
  activeDevice: DeviceProfile | null;
  connectionPhase: ConnectionPhase;
  scanningAdb: boolean;
  terminalTabCount: number;
  runningServiceCount: number;
  onSelectDevice: (id: string) => void;
  onRemoveDevice: (id: string) => void;
  onOpenAddDevice: () => void;
  onScanAdb: () => void;
}

const NAV_ITEMS: Array<{
  id: NavViewId;
  label: string;
  icon: typeof Box;
}> = [
  { id: 'containers', label: '容器', icon: Box },
  { id: 'services', label: '服务', icon: ServerCog },
  { id: 'terminal', label: '终端', icon: Terminal },
  { id: 'install', label: '安装与导入', icon: Download },
  { id: 'network', label: '网络与 SSH', icon: Network },
  { id: 'logs', label: '日志与内核', icon: Cpu },
];

function connectionMeta(phase: ConnectionPhase): {
  label: string;
  variant: 'success' | 'warning' | 'danger' | 'neutral';
} {
  switch (phase) {
    case 'connected':
      return { label: '已连接', variant: 'success' };
    case 'connecting':
      return { label: '连接中', variant: 'warning' };
    case 'error':
      return { label: '连接异常', variant: 'danger' };
    case 'disconnected':
      return { label: '未连接', variant: 'neutral' };
  }
}

/**
 * Collapsible desktop sidebar featuring multi-device switcher and 6 navigation views.
 */
export function Sidebar({
  collapsed,
  onToggleCollapse,
  activeView,
  onSelectView,
  devices,
  activeDevice,
  connectionPhase,
  scanningAdb,
  terminalTabCount,
  runningServiceCount,
  onSelectDevice,
  onRemoveDevice,
  onOpenAddDevice,
  onScanAdb,
}: SidebarProps) {
  const conn = connectionMeta(connectionPhase);

  return (
    <aside
      className={cn(
        'flex h-full shrink-0 flex-col border-r border-[var(--border-hairline)] bg-[var(--surface-elevated)] transition-[width] duration-150',
        collapsed ? 'w-[68px]' : 'w-[252px]',
      )}
    >
      {/* Brand header */}
      <div className="flex items-center justify-between gap-2 border-b border-[var(--border-hairline)] px-3 py-3">
        <div className="flex min-w-0 items-center gap-2">
          <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[var(--accent)]/15 text-[var(--accent)]">
            <Layers className="size-4" />
          </div>
          {!collapsed && (
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold text-[var(--text-primary)]">
                Strata Desktop
              </div>
              <div className="truncate text-[11px] text-[var(--text-secondary)]">
                Android 容器控制中心
              </div>
            </div>
          )}
        </div>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={collapsed ? '展开侧边栏' : '折叠侧边栏'}
          onClick={onToggleCollapse}
        >
          {collapsed ? <PanelLeftOpen className="size-4" /> : <PanelLeftClose className="size-4" />}
        </Button>
      </div>

      {/* Multi-device switcher */}
      <div className="border-b border-[var(--border-hairline)] p-2.5">
        {!collapsed ? (
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between gap-1">
              <span className="text-[11px] font-medium text-[var(--text-secondary)]">
                目标设备 ({devices.length})
              </span>
              <Badge variant={conn.variant}>{conn.label}</Badge>
            </div>
            <div className="flex max-h-32 flex-col gap-1 overflow-y-auto strata-scroll">
              {devices.map((dev) => {
                const selected = activeDevice?.id === dev.id;
                return (
                  <div
                    key={dev.id}
                    className={cn(
                      'group flex items-center gap-1 rounded-lg border px-2 py-1.5 text-left transition-colors',
                      selected
                        ? 'border-[var(--accent)]/45 bg-[var(--accent)]/12 text-[var(--text-primary)]'
                        : 'border-transparent hover:bg-[var(--surface-card)] text-[var(--text-secondary)]',
                    )}
                  >
                    <button
                      type="button"
                      onClick={() => onSelectDevice(dev.id)}
                      className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    >
                      {dev.kind === 'lan' ? (
                        <Wifi className="size-3.5 shrink-0 text-[var(--accent)]" />
                      ) : (
                        <Smartphone className="size-3.5 shrink-0 text-[var(--accent)]" />
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-xs font-medium">{dev.label}</div>
                        <div className="truncate font-mono text-[10px] opacity-75">
                          {dev.kind === 'adb' ? dev.serial : `${dev.host}:${dev.port}`}
                        </div>
                      </div>
                    </button>
                    {devices.length > 1 && (
                      <button
                        type="button"
                        aria-label={`移除设备 ${dev.label}`}
                        onClick={() => onRemoveDevice(dev.id)}
                        className="opacity-0 transition-opacity hover:text-[var(--danger)] group-hover:opacity-100"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
            <div className="flex items-center gap-1.5">
              <Button size="sm" variant="secondary" className="flex-1" onClick={onOpenAddDevice}>
                <Plus className="size-3.5" />
                扫描 ADB / 添加
              </Button>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label="刷新 ADB 设备"
                disabled={scanningAdb}
                onClick={onScanAdb}
              >
                <RefreshCw className={cn('size-3.5', scanningAdb && 'animate-spin')} />
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-1.5">
            <Button
              size="icon-sm"
              variant="secondary"
              aria-label="扫描 ADB / 添加设备"
              onClick={onOpenAddDevice}
            >
              <Smartphone className="size-4" />
            </Button>
          </div>
        )}
      </div>

      {/* Primary navigation */}
      <nav
        className="flex flex-1 flex-col gap-1 overflow-y-auto p-2 strata-scroll"
        aria-label="主导航"
      >
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const active = activeView === item.id;
          const counter =
            item.id === 'terminal' && terminalTabCount > 0
              ? terminalTabCount
              : item.id === 'services' && runningServiceCount > 0
                ? runningServiceCount
                : null;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => onSelectView(item.id)}
              title={collapsed ? item.label : undefined}
              className={cn(
                'flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-xs font-medium transition-colors',
                collapsed && 'justify-center px-0',
                active
                  ? 'bg-[var(--accent)]/15 text-[var(--accent)]'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--surface-card)] hover:text-[var(--text-primary)]',
              )}
            >
              <Icon className="size-4 shrink-0" />
              {!collapsed && <span className="flex-1 truncate text-left">{item.label}</span>}
              {!collapsed && counter !== null && (
                <Badge variant={active ? 'accent' : 'neutral'}>{counter}</Badge>
              )}
            </button>
          );
        })}
      </nav>

      {/* Footer theme controls */}
      {!collapsed && (
        <div className="border-t border-[var(--border-hairline)] p-2.5">
          <ThemeSwitcher compact />
        </div>
      )}
    </aside>
  );
}
