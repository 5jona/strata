import { Badge, Button, Card, Input, SegmentedControl, StatusBanner, Switch } from '@gojona/ui';
import { RefreshCw, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import type { StrataTransport } from '../../transport/httpWsTransport.ts';
import type { ToolManagerKind, ToolUseScope, ToolVersionOption } from '../../types/protocol.ts';
import { STRATA_METHODS } from '../../types/rpc.ts';

export interface VersionPickerTarget {
  mode: 'install' | 'use';
  manager: ToolManagerKind;
  name: string;
  defaultVersion?: string;
}

export interface ToolVersionPickerModalProps {
  target: VersionPickerTarget | null;
  container: string;
  transport: StrataTransport | null;
  configPaths: string[];
  busy: boolean;
  onClose: () => void;
  onInstallVersion: (manager: ToolManagerKind, spec: string, pin: boolean) => void;
  onUseVersion: (
    manager: ToolManagerKind,
    name: string,
    version: string,
    scope: ToolUseScope,
    configPath?: string,
  ) => void;
}

/** Modal querying `tools.versions` and letting users freely choose or type a version. */
export function ToolVersionPickerModal({
  target,
  container,
  transport,
  configPaths,
  busy,
  onClose,
  onInstallVersion,
  onUseVersion,
}: ToolVersionPickerModalProps) {
  const [versions, setVersions] = useState<ToolVersionOption[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [includePrerelease, setIncludePrerelease] = useState(false);
  const [selectedVersion, setSelectedVersion] = useState('');
  const [pinGlobal, setPinGlobal] = useState(true);
  const [scope, setScope] = useState<ToolUseScope>('global');
  const [configPath, setConfigPath] = useState('');

  const supportsRemoteVersions = target?.manager === 'mise' || target?.manager === 'uv';

  const fetchVersions = useCallback(async () => {
    if (!target || !transport || !supportsRemoteVersions) return;
    setLoading(true);
    setError(null);
    try {
      const res = await transport.invoke(STRATA_METHODS.TOOLS_VERSIONS, {
        container,
        manager: target.manager,
        name: target.name,
        includePrerelease,
      });
      setVersions(res.versions);
      setSelectedVersion(
        (prev) => prev || res.versions[0]?.version || target.defaultVersion || 'latest',
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [container, includePrerelease, supportsRemoteVersions, target, transport]);

  useEffect(() => {
    if (!target) return;
    setSelectedVersion(target.defaultVersion ?? '');
    setVersions([]);
    setError(null);
  }, [target]);

  useEffect(() => {
    if (!target) return;
    setConfigPath((prev) => (prev && configPaths.includes(prev) ? prev : (configPaths[0] ?? '')));
  }, [configPaths, target]);

  useEffect(() => {
    if (!target) return;
    void fetchVersions();
  }, [fetchVersions, target]);

  if (!target) return null;

  const trimmedVer = selectedVersion.trim();
  const handleConfirm = () => {
    if (!trimmedVer) return;
    if (target.mode === 'use') {
      onUseVersion(
        target.manager,
        target.name,
        trimmedVer,
        scope,
        scope === 'global' && configPath ? configPath : undefined,
      );
    } else {
      const spec =
        target.manager === 'mise'
          ? `${target.name}@${trimmedVer}`
          : target.manager === 'uv' && trimmedVer !== 'latest'
            ? `${target.name}==${trimmedVer}`
            : target.name;
      onInstallVersion(target.manager, spec, pinGlobal);
    }
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
    >
      <Card className="flex max-h-[86vh] w-full max-w-lg flex-col gap-3 overflow-hidden p-5">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              {target.mode === 'use' ? '切换生效版本' : '选择版本安装'} · {target.name}
            </h3>
            <p className="text-xs text-[var(--text-secondary)]">
              管理器：{target.manager} · 支持点击候选版本或手动输入任意版本号 / 标签（如
              lts、latest）
            </p>
          </div>
          <Button size="icon-sm" variant="ghost" aria-label="关闭" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium text-[var(--text-secondary)]">目标版本</span>
            {supportsRemoteVersions && (
              <div className="flex items-center gap-2">
                <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
                  <Switch checked={includePrerelease} onCheckedChange={setIncludePrerelease} />
                  <span>显示预览版</span>
                </div>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  disabled={loading}
                  onClick={() => void fetchVersions()}
                >
                  <RefreshCw className={`size-3.5 ${loading ? 'animate-spin' : ''}`} />
                </Button>
              </div>
            )}
          </div>
          <Input
            mono
            value={selectedVersion}
            placeholder="例如：3.12.9、lts、latest"
            onChange={(e) => setSelectedVersion(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleConfirm();
            }}
          />
        </div>

        {error && (
          <StatusBanner
            tone="warning"
            title="远程版本拉取失败，可手动输入版本号"
            description={error}
          />
        )}

        {supportsRemoteVersions && (
          <div className="flex max-h-52 flex-col gap-1 overflow-y-auto rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-2 strata-scroll">
            {loading && versions.length === 0 ? (
              <span className="py-6 text-center text-xs text-[var(--text-secondary)]">
                正在查询可用版本…
              </span>
            ) : versions.length === 0 ? (
              <span className="py-4 text-center text-xs text-[var(--text-secondary)]">
                暂无候选列表，请在上方直接输入版本
              </span>
            ) : (
              versions.slice(0, 60).map((opt) => {
                const picked = trimmedVer === opt.version;
                return (
                  <button
                    key={opt.version}
                    type="button"
                    onClick={() => setSelectedVersion(opt.version)}
                    className={`flex items-center justify-between rounded-md px-2.5 py-1.5 text-left text-xs transition-colors ${
                      picked
                        ? 'bg-[var(--accent)]/15 text-[var(--accent)]'
                        : 'text-[var(--text-primary)] hover:bg-[var(--surface-card)]'
                    }`}
                  >
                    <span className="font-mono font-medium">{opt.version}</span>
                    <div className="flex items-center gap-1.5">
                      {opt.prerelease && <Badge variant="warning">预览版</Badge>}
                      {opt.installed && <Badge variant="neutral">已安装</Badge>}
                      {opt.active && <Badge variant="success">当前激活</Badge>}
                      {opt.createdAt && (
                        <span className="font-mono text-[10px] text-[var(--text-secondary)]">
                          {opt.createdAt.slice(0, 10)}
                        </span>
                      )}
                    </div>
                  </button>
                );
              })
            )}
          </div>
        )}

        {target.mode === 'use' ? (
          <div className="flex flex-col gap-2 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-2.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-xs text-[var(--text-secondary)]">生效作用域</span>
              <SegmentedControl
                size="sm"
                value={scope}
                onValueChange={setScope}
                options={[
                  { value: 'global', label: '全局默认 (global)' },
                  { value: 'workspace', label: '工作区 (/workspace)' },
                ]}
              />
            </div>
            {scope === 'global' && configPaths.length > 1 && (
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-[11px] text-[var(--text-secondary)]">写入配置：</span>
                {configPaths.map((p) => (
                  <Button
                    key={p}
                    size="sm"
                    variant={configPath === p ? 'pill' : 'ghost'}
                    onClick={() => setConfigPath(p)}
                  >
                    {p}
                  </Button>
                ))}
              </div>
            )}
          </div>
        ) : (
          target.manager === 'mise' && (
            <div className="flex items-center justify-between rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2">
              <span className="text-xs text-[var(--text-primary)]">
                安装后保存到全局工具清单并激活
              </span>
              <Switch checked={pinGlobal} onCheckedChange={setPinGlobal} />
            </div>
          )
        )}

        <div className="flex justify-end gap-2 pt-1">
          <Button size="sm" variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={busy || !trimmedVer}
            onClick={handleConfirm}
          >
            {target.mode === 'use' ? '切换并生效' : '确认安装'}
          </Button>
        </div>
      </Card>
    </div>
  );
}
