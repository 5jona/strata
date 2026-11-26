import { Badge, Button, Card, Input, SegmentedControl, StatusBanner, Switch } from '@gojona/ui';
import { Download, Plus, Search } from 'lucide-react';
import { useEffect, useState } from 'react';
import type {
  ToolCatalogEntry,
  ToolGroupKind,
  ToolManagerKind,
  ToolsCatalogResult,
} from '../../types/protocol.ts';
import { ToolsBundlePresets } from './ToolsBundlePresets.tsx';

const FORBIDDEN_PATTERN =
  /^(openssh|openssh-server|openssh-sftp-server-privsep|sshd|sudo|policykit-1)([=@].*)?$/i;

const CATALOG_GROUPS: Array<{ value: 'all' | ToolGroupKind; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'mise-runtimes', label: '编程语言运行时' },
  { value: 'shell', label: '终端增强 (Shell)' },
  { value: 'cli-dev', label: '开发 CLI 工具' },
  { value: 'python-uv', label: 'Python / uv' },
  { value: 'debug-l3-l5', label: '系统诊断工具' },
];

export interface ToolsCatalogTabProps {
  containerId: string;
  catalog: ToolsCatalogResult | null;
  systemPkgManager?: string;
  containerRunning: boolean;
  busy: boolean;
  onSearchCatalog: (query: string) => void;
  onInstall: (manager: ToolManagerKind, packages: string[], pin: boolean) => void;
  onOpenVersionPicker: (manager: ToolManagerKind, name: string, defaultVersion: string) => void;
  onOpenTerminal?: (containerId: string) => void;
}

/** Dynamic `mise registry` search, manual exact package installer, and version-selectable catalog. */
export function ToolsCatalogTab({
  containerId,
  catalog,
  systemPkgManager,
  containerRunning,
  busy,
  onSearchCatalog,
  onInstall,
  onOpenVersionPicker,
  onOpenTerminal,
}: ToolsCatalogTabProps) {
  const [query, setQuery] = useState('');
  const [group, setGroup] = useState<'all' | ToolGroupKind>('all');
  const sysMgr: ToolManagerKind =
    systemPkgManager === 'apk' || systemPkgManager === 'pacman' || systemPkgManager === 'dnf'
      ? systemPkgManager
      : 'apt';
  const [manualMgr, setManualMgr] = useState<ToolManagerKind>('mise');
  const [manualPkg, setManualPkg] = useState('');
  const [pinSpec, setPinSpec] = useState(true);
  const [blockError, setBlockError] = useState<string | null>(null);

  useEffect(() => {
    const timer = setTimeout(() => onSearchCatalog(query), 280);
    return () => clearTimeout(timer);
  }, [onSearchCatalog, query]);

  const disabled = busy || !containerRunning;
  const q = query.trim().toLowerCase();
  const entries = (catalog?.entries ?? []).filter((entry) => {
    if (group !== 'all' && entry.group !== group) return false;
    if (!q) return true;
    return (
      entry.id.toLowerCase().includes(q) ||
      entry.name.toLowerCase().includes(q) ||
      entry.summary.toLowerCase().includes(q) ||
      entry.tags.some((t) => t.toLowerCase().includes(q))
    );
  });

  const handleManualInstall = () => {
    const specs = manualPkg
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (specs.length === 0) return;
    const blocked = specs.find((s) => FORBIDDEN_PATTERN.test(s));
    if (blocked) {
      setBlockError(
        `已拦截受限包 ${blocked}：受容器环境安全策略限制，禁止安装 openssh-server、sshd 或 sudo`,
      );
      return;
    }
    setBlockError(null);
    onInstall(manualMgr, specs, pinSpec);
    setManualPkg('');
  };

  return (
    <div className="flex flex-col gap-4">
      <StatusBanner
        tone="info"
        title="兼容提示：请勿安装 openssh-server / sshd / sudo"
        description="容器已内置轻量 Dropbear 公钥 SSH 服务；安装传统提权守护进程会触发系统安全策略拦截。"
      />

      <ToolsBundlePresets
        containerId={containerId}
        bundles={catalog?.bundles ?? []}
        disabled={disabled}
        onInstallBundle={onInstall}
        {...(onOpenTerminal ? { onOpenTerminal } : {})}
      />

      <Card className="flex flex-col gap-2.5 p-4">
        <span className="text-xs font-semibold text-[var(--text-primary)]">
          手动安装指定工具或版本（支持选版本或直接输入如 python@3.13）
        </span>
        {blockError && <StatusBanner tone="danger" title={blockError} />}
        <div className="flex flex-wrap items-center gap-2">
          <SegmentedControl
            size="sm"
            value={manualMgr}
            onValueChange={setManualMgr}
            options={[
              { value: 'mise', label: 'mise' },
              { value: 'uv', label: 'uv' },
              { value: sysMgr, label: sysMgr },
            ]}
          />
          <Input
            mono
            className="min-w-[220px] flex-1"
            value={manualPkg}
            placeholder={
              manualMgr === 'mise'
                ? '例如：python@3.13 或 aqua:jqlang/jq'
                : manualMgr === 'uv'
                  ? '例如：ruff httpie'
                  : '例如：strace sqlite3'
            }
            onChange={(e) => setManualPkg(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleManualInstall();
            }}
          />
          {manualMgr === 'mise' && (
            <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
              <Switch checked={pinSpec} onCheckedChange={setPinSpec} />
              <span>保存到全局工具清单</span>
            </div>
          )}
          {(manualMgr === 'mise' || manualMgr === 'uv') &&
            manualPkg.trim() &&
            !manualPkg.includes('@') && (
              <Button
                size="sm"
                variant="secondary"
                disabled={disabled}
                onClick={() =>
                  onOpenVersionPicker(
                    manualMgr,
                    manualPkg.trim().split(/[\s,]+/)[0] ?? '',
                    'latest',
                  )
                }
              >
                选版本
              </Button>
            )}
          <Button
            size="sm"
            variant="primary"
            disabled={disabled || !manualPkg.trim()}
            onClick={handleManualInstall}
          >
            <Plus className="size-3.5" />
            立即安装
          </Button>
        </div>
      </Card>

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-1">
          {CATALOG_GROUPS.map((g) => (
            <Button
              key={g.value}
              size="sm"
              variant={group === g.value ? 'pill' : 'ghost'}
              onClick={() => setGroup(g.value)}
            >
              {g.label}
            </Button>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <Badge variant={catalog?.registryAvailable ? 'success' : 'neutral'}>
            {catalog?.registryAvailable ? '已连接 mise 全量注册表' : '离线精选目录'}
          </Badge>
          <div className="relative w-64">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-[var(--text-secondary)]" />
            <Input
              className="pl-8"
              value={query}
              placeholder="搜索工具名、标签或 mise registry…"
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
        {entries.map((entry: ToolCatalogEntry) => (
          <Card
            key={`${entry.manager}:${entry.id}`}
            className="flex flex-col justify-between gap-2.5 p-3.5"
          >
            <div className="flex flex-col gap-1">
              <div className="flex items-center justify-between gap-2">
                <span className="text-sm font-semibold text-[var(--text-primary)]">
                  {entry.name}
                </span>
                <div className="flex items-center gap-1">
                  {entry.recommended && <Badge variant="success">推荐</Badge>}
                  <Badge variant="accent">{entry.manager}</Badge>
                </div>
              </div>
              <p className="text-xs text-[var(--text-secondary)]">{entry.summary}</p>
              <div className="flex items-center justify-between font-mono text-[11px] text-[var(--text-secondary)]">
                <span>{entry.backendHint}</span>
                <span>{entry.estimatedMb > 0 ? `~${entry.estimatedMb} MiB` : '在线工具'}</span>
              </div>
            </div>
            <div className="flex items-center gap-1.5">
              <Button
                size="sm"
                variant="primary"
                className="flex-1"
                disabled={disabled}
                onClick={() => {
                  const pkg =
                    entry.manager === 'mise' && entry.defaultVersion !== 'system'
                      ? `${entry.id}@${entry.defaultVersion}`
                      : entry.id;
                  onInstall(entry.manager, [pkg], true);
                }}
              >
                <Download className="size-3.5" />
                安装 ({entry.defaultVersion})
              </Button>
              {(entry.manager === 'mise' || entry.manager === 'uv') && (
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={disabled}
                  onClick={() => onOpenVersionPicker(entry.manager, entry.id, entry.defaultVersion)}
                >
                  选版本
                </Button>
              )}
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}
