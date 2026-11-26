import { Badge, Button, cn, ThemeSwitcher } from '@gojona/ui';
import {
  Box,
  Cpu,
  Layers,
  Network,
  PanelLeftClose,
  PanelLeftOpen,
  Pencil,
  Plus,
  RefreshCw,
  ServerCog,
  Smartphone,
  Terminal,
  Trash2,
  Wifi,
  Wrench,
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
  onEditDevice: (device: DeviceProfile) => void;
  onRemoveDevice: (id: string) => void;
  onOpenAddDevice: () => void;
  onScanAdb: () => void;
}

const NAV_GROUPS: Array<{
  title: string;
  items: Array<{ id: NavViewId; label: string; icon: typeof Box }>;
}> = [
  {
    title: '工作区',
    items: [
      { id: 'containers', label: '容器', icon: Box },
      { id: 'terminal', label: '终端', icon: Terminal },
      { id: 'services', label: '服务', icon: ServerCog },
      { id: 'tools', label: '工具环境', icon: Wrench },
    ],
  },
  {
    title: '连接与运维',
    items: [
      { id: 'network', label: '网络与安全', icon: Network },
      { id: 'logs', label: '日志与内核', icon: Cpu },
    ],
  },
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
 * Collapsible desktop sidebar featuring multi-device switcher (with edit/token action)
 * and 2-group 6-item workspace navigation.
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
  onEditDevice,
  onRemoveDevice,
  onOpenAddDevice,
  onScanAdb,
}: SidebarProps) {
  const conn = connectionMeta(connectionPhase);
  const navDisabled = !activeDevice;

  return (
    <aside
      className={cn(
        'flex h-full shrink-0 flex-col border-r border-[var(--border-hairline)] bg-[var(--surface-elevated)] transition-[width] duration-150',
        collapsed ? 'w-[68px]' : 'w-[256px]',
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
            <div className="flex max-h-36 flex-col gap-1 overflow-y-auto strata-scroll">
              {devices.map((dev) => {
                const selected = dev.id === activeDevice?.id;
                return (
                  <div
                    key={dev.id}
                    className={cn(
                      'group flex items-center gap-1.5 rounded-lg border px-2 py-1.5 transition-colors',
                      selected
                        ? 'border-[var(--accent)]/40 bg-[var(--accent)]/10 text-[var(--text-primary)]'
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
                    <button
                      type="button"
                      title="编辑设备配置 / 补充 Token"
                      aria-label={`编辑设备 ${dev.label}`}
                      onClick={() => onEditDevice(dev)}
                      className={cn(
                        'transition-opacity hover:text-[var(--accent)]',
                        selected && connectionPhase === 'error'
                          ? 'opacity-100 text-[var(--warning)]'
                          : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100',
                      )}
                    >
                      <Pencil className="size-3.5" />
                    </button>
                    {devices.length > 1 && (
                      <button
                        type="button"
                        title="移除设备"
                        aria-label={`移除设备 ${dev.label}`}
                        onClick={() => onRemoveDevice(dev.id)}
                        className="opacity-0 transition-opacity hover:text-[var(--danger)] group-hover:opacity-100 focus-visible:opacity-100"
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

      {/* Primary grouped navigation (6 items in 2 groups) */}
      <nav
        className="flex flex-1 flex-col gap-3 overflow-y-auto p-2 strata-scroll"
        aria-label="主导航"
      >
        {!collapsed && navDisabled && (
          <div className="rounded-lg border border-dashed border-[var(--border-hairline)] px-2.5 py-2 text-[11px] text-[var(--text-secondary)]">
            连接 Android 设备后即可启用工作区功能
          </div>
        )}
        {NAV_GROUPS.map((group) => (
          <div key={group.title} className="flex flex-col gap-1">
            {!collapsed && (
              <div className="px-2.5 pt-0.5 text-[10px] font-semibold tracking-wider text-[var(--text-secondary)]/80 uppercase">
                {group.title}
              </div>
            )}
            {group.items.map((item) => {
              const Icon = item.icon;
              const active =
                activeView === item.id || (item.id === 'containers' && activeView === 'install');
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
                  disabled={navDisabled}
                  onClick={() => onSelectView(item.id)}
                  title={collapsed ? item.label : undefined}
                  className={cn(
                    'flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-xs font-medium transition-colors',
                    collapsed && 'justify-center px-0',
                    navDisabled && 'cursor-not-allowed opacity-45',
                    active && !navDisabled
                      ? 'bg-[var(--accent)]/15 text-[var(--accent)]'
                      : 'text-[var(--text-secondary)] hover:bg-[var(--surface-card)] hover:text-[var(--text-primary)]',
                  )}
                >
                  <Icon className="size-4 shrink-0" />
                  {!collapsed && <span className="flex-1 truncate text-left">{item.label}</span>}
                  {!collapsed && counter !== null && !navDisabled && (
                    <Badge variant={active ? 'accent' : 'neutral'}>{counter}</Badge>
                  )}
                </button>
              );
            })}
          </div>
        ))}
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
