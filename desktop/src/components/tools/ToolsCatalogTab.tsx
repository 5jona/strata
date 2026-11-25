import { Badge, Button, Card, Input, SegmentedControl, StatusBanner, Switch } from '@gojona/ui';
import { Download, Layers, Plus, Search } from 'lucide-react';
import { useState } from 'react';
import type {
  ToolBundleSpec,
  ToolCatalogEntry,
  ToolGroupKind,
  ToolManagerKind,
  ToolsCatalogResult,
} from '../../types/protocol.ts';

const FORBIDDEN_PATTERN =
  /^(openssh-server|openssh-sftp-server-privsep|sshd|sudo|policykit-1)([=@].*)?$/i;

const CATALOG_GROUPS: Array<{ value: 'all' | ToolGroupKind; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'mise-runtimes', label: '运行时' },
  { value: 'cli-dev', label: 'CLI 开发' },
  { value: 'debug-l3-l5', label: 'L3-L5 调试' },
  { value: 'python-uv', label: 'Python / uv' },
];

export interface ToolsCatalogTabProps {
  catalog: ToolsCatalogResult | null;
  busy: boolean;
  onInstall: (manager: ToolManagerKind, packages: string[], pin: boolean) => void;
}

/**
 * Curated ARM64/GKI-safe bundles, searchable tool catalog, and manual package installer.
 */
export function ToolsCatalogTab({ catalog, busy, onInstall }: ToolsCatalogTabProps) {
  const [query, setQuery] = useState('');
  const [group, setGroup] = useState<'all' | ToolGroupKind>('all');
  const [manualMgr, setManualMgr] = useState<'mise' | 'uv' | 'apt'>('mise');
  const [manualPkg, setManualPkg] = useState('');
  const [pinSpec, setPinSpec] = useState(true);
  const [blockError, setBlockError] = useState<string | null>(null);

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
        `已拦截危险包 ${blocked}：ColorOS Watchdog 严禁安装 openssh-server / sshd / sudo`,
      );
      return;
    }
    setBlockError(null);
    onInstall(manualMgr, specs, pinSpec);
    setManualPkg('');
  };

  const bundles: ToolBundleSpec[] = catalog?.bundles ?? [];

  return (
    <div className="flex flex-col gap-4">
      <StatusBanner
        tone="warning"
        title="ColorOS Watchdog 安全红线：严禁安装 openssh-server / sshd / sudo"
        description="容器已内置仅公钥 Dropbear SSH 与 sftp-server；安装 OpenSSH privsep 守护进程将触发内核安全卫士强杀。"
      />

      {/* Curated Bundles */}
      {bundles.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
            <Layers className="size-3.5 text-[var(--accent)]" />
            <span>ARM64 精选组合包</span>
          </div>
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-4">
            {bundles.map((bundle) => (
              <Card key={bundle.id} className="flex flex-col justify-between gap-2.5 p-3.5">
                <div className="flex flex-col gap-1">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-sm font-semibold text-[var(--text-primary)]">
                      {bundle.name}
                    </span>
                    <Badge variant="accent">{bundle.tools.length} 项</Badge>
                  </div>
                  <p className="text-xs text-[var(--text-secondary)]">{bundle.summary}</p>
                </div>
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={busy}
                  onClick={() => onInstall('bundle', [bundle.id], true)}
                >
                  <Download className="size-3.5" />
                  一键安装
                </Button>
              </Card>
            ))}
          </div>
        </div>
      )}

      {/* Manual Exact Package Install */}
      <Card className="flex flex-col gap-2.5 p-4">
        <span className="text-xs font-semibold text-[var(--text-primary)]">
          手动精确安装（mise / uv / apt）
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
              { value: 'apt', label: 'apt' },
            ]}
          />
          <Input
            mono
            className="min-w-[220px] flex-1"
            value={manualPkg}
            placeholder={
              manualMgr === 'mise'
                ? '例如：python@3.12 或 aqua:jqlang/jq'
                : manualMgr === 'uv'
                  ? '例如：ruff httpie'
                  : '例如：strace sqlite3'
            }
            onChange={(e) => setManualPkg(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleManualInstall();
            }}
          />
          <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
            <Switch checked={pinSpec} onCheckedChange={setPinSpec} />
            <span>写入全局配置</span>
          </div>
          <Button
            size="sm"
            variant="primary"
            disabled={busy || !manualPkg.trim()}
            onClick={handleManualInstall}
          >
            <Plus className="size-3.5" />
            立即安装
          </Button>
        </div>
      </Card>

      {/* Catalog Search + Entries */}
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
        <div className="relative w-64">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-[var(--text-secondary)]" />
          <Input
            className="pl-8"
            value={query}
            placeholder="搜索工具名或标签…"
            onChange={(e) => setQuery(e.target.value)}
          />
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
                <span>~{entry.estimatedMb} MiB</span>
              </div>
            </div>
            <Button
              size="sm"
              variant="primary"
              disabled={busy}
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
          </Card>
        ))}
      </div>
    </div>
  );
}
