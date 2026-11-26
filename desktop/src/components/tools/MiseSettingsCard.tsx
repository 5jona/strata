import { Badge, Button, Card, Input, SegmentedControl } from '@gojona/ui';
import { KeyRound, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';

export interface UrlReplacementRow {
  uid: string;
  from: string;
  to: string;
}

export interface MiseSettingsCardProps {
  configPathLabel: string;
  editMode: 'visual' | 'toml';
  jobs: string;
  minimumReleaseAge: string;
  urlReplRows: UrlReplacementRow[];
  miseConfigToml: string;
  hasGithubToken: boolean;
  githubTokenMasked?: string;
  githubTokenInput: string;
  containerRunning: boolean;
  busy: boolean;
  onEditModeChange: (mode: 'visual' | 'toml') => void;
  onJobsChange: (val: string) => void;
  onMinimumReleaseAgeChange: (val: string) => void;
  onUrlReplRowsChange: (rows: UrlReplacementRow[]) => void;
  onTomlChange: (val: string) => void;
  onGithubTokenInputChange: (val: string) => void;
  onClearGithubToken: () => void;
}

/** Token credential card + visual `mise.toml` settings vs raw TOML editor. */
export function MiseSettingsCard({
  configPathLabel,
  editMode,
  jobs,
  minimumReleaseAge,
  urlReplRows,
  miseConfigToml,
  hasGithubToken,
  githubTokenMasked,
  githubTokenInput,
  containerRunning,
  busy,
  onEditModeChange,
  onJobsChange,
  onMinimumReleaseAgeChange,
  onUrlReplRowsChange,
  onTomlChange,
  onGithubTokenInputChange,
  onClearGithubToken,
}: MiseSettingsCardProps) {
  const [newFrom, setNewFrom] = useState('');
  const [newTo, setNewTo] = useState('');

  return (
    <>
      <Card className="flex flex-col gap-2.5 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <KeyRound className="size-4 text-[var(--accent)]" />
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              MISE_GITHUB_TOKEN（脱敏凭据）
            </span>
          </div>
          <Badge variant={hasGithubToken ? 'success' : 'neutral'}>
            {hasGithubToken ? `已配置 (${githubTokenMasked ?? '****'})` : '未配置'}
          </Badge>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            mono
            type="password"
            className="min-w-[240px] flex-1"
            value={githubTokenInput}
            placeholder="输入新 GitHub Token 以更新（服务端永不回显明文）"
            onChange={(e) => onGithubTokenInputChange(e.target.value)}
          />
          {hasGithubToken && (
            <Button
              size="sm"
              variant="danger"
              disabled={busy || !containerRunning}
              onClick={onClearGithubToken}
            >
              <Trash2 className="size-3.5" />
              清空 Token
            </Button>
          )}
        </div>
      </Card>

      <Card className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="font-mono text-xs font-semibold text-[var(--text-primary)]">
            {configPathLabel}
          </span>
          <SegmentedControl
            size="sm"
            value={editMode}
            onValueChange={onEditModeChange}
            options={[
              { value: 'visual', label: '可视化配置' },
              { value: 'toml', label: '高级 TOML 源码' },
            ]}
          />
        </div>

        {editMode === 'visual' ? (
          <div className="flex flex-col gap-3">
            <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">并发任务数 (jobs)</span>
                <Input mono value={jobs} onChange={(e) => onJobsChange(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">
                  最小发布缓冲期 (minimum_release_age)
                </span>
                <Input
                  mono
                  value={minimumReleaseAge}
                  placeholder="24h"
                  onChange={(e) => onMinimumReleaseAgeChange(e.target.value)}
                />
              </div>
            </div>

            <div className="flex flex-col gap-2">
              <span className="text-xs font-medium text-[var(--text-secondary)]">
                URL 域名替换规则 ([settings.url_replacements])
              </span>
              {urlReplRows.map((row) => (
                <div key={row.uid} className="flex items-center gap-2">
                  <Input
                    mono
                    value={row.from}
                    placeholder="原始域名，如 github.com"
                    onChange={(e) =>
                      onUrlReplRowsChange(
                        urlReplRows.map((r) =>
                          r.uid === row.uid ? { ...r, from: e.target.value } : r,
                        ),
                      )
                    }
                  />
                  <span className="text-xs text-[var(--text-secondary)]">→</span>
                  <Input
                    mono
                    value={row.to}
                    placeholder="目标镜像域名"
                    onChange={(e) =>
                      onUrlReplRowsChange(
                        urlReplRows.map((r) =>
                          r.uid === row.uid ? { ...r, to: e.target.value } : r,
                        ),
                      )
                    }
                  />
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label="删除规则"
                    onClick={() =>
                      onUrlReplRowsChange(urlReplRows.filter((r) => r.uid !== row.uid))
                    }
                  >
                    <Trash2 className="size-3.5 text-[var(--danger)]" />
                  </Button>
                </div>
              ))}
              <div className="flex items-center gap-2">
                <Input
                  mono
                  value={newFrom}
                  placeholder="新增原始域名 (如 github.com)"
                  onChange={(e) => setNewFrom(e.target.value)}
                />
                <Input
                  mono
                  value={newTo}
                  placeholder="替换为 (如 mirror.example.com)"
                  onChange={(e) => setNewTo(e.target.value)}
                />
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={!newFrom.trim() || !newTo.trim()}
                  onClick={() => {
                    onUrlReplRowsChange([
                      ...urlReplRows,
                      {
                        uid: `repl-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`,
                        from: newFrom.trim(),
                        to: newTo.trim(),
                      },
                    ]);
                    setNewFrom('');
                    setNewTo('');
                  }}
                >
                  <Plus className="size-3.5" />
                  添加
                </Button>
              </div>
            </div>
          </div>
        ) : (
          <textarea
            rows={10}
            value={miseConfigToml}
            onChange={(e) => onTomlChange(e.target.value)}
            className="w-full rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
          />
        )}
      </Card>
    </>
  );
}
