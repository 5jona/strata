import { Badge, Button, Card, Input, SegmentedControl, StatusBanner } from '@gojona/ui';
import { KeyRound, Save, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import type {
  ToolMirrorPreset,
  ToolsConfigResult,
  ToolsConfigSetParams,
} from '../../types/protocol.ts';

export interface ToolsConfigTabProps {
  config: ToolsConfigResult | null;
  busy: boolean;
  onSave: (params: ToolsConfigSetParams) => Promise<void>;
}

/**
 * Mirror preset, proxy, `/etc/strata/mise/config.toml`, and masked `MISE_GITHUB_TOKEN` editor.
 */
export function ToolsConfigTab({ config, busy, onSave }: ToolsConfigTabProps) {
  const [mirrorPreset, setMirrorPreset] = useState<ToolMirrorPreset>('tuna');
  const [aptMirrorUrl, setAptMirrorUrl] = useState('');
  const [nodeMirrorUrl, setNodeMirrorUrl] = useState('');
  const [pypiIndexUrl, setPypiIndexUrl] = useState('');
  const [npmRegistryUrl, setNpmRegistryUrl] = useState('');
  const [rustupDistServer, setRustupDistServer] = useState('');
  const [goproxy, setGoproxy] = useState('');
  const [httpProxy, setHttpProxy] = useState('');
  const [httpsProxy, setHttpsProxy] = useState('');
  const [noProxy, setNoProxy] = useState('');
  const [jobs, setJobs] = useState('4');
  const [minimumReleaseAge, setMinimumReleaseAge] = useState('24h');
  const [githubTokenInput, setGithubTokenInput] = useState('');
  const [miseConfigToml, setMiseConfigToml] = useState('');

  useEffect(() => {
    if (!config) return;
    setMirrorPreset(config.mirrorPreset);
    setAptMirrorUrl(config.aptMirrorUrl ?? '');
    setNodeMirrorUrl(config.nodeMirrorUrl ?? '');
    setPypiIndexUrl(config.pypiIndexUrl ?? '');
    setNpmRegistryUrl(config.npmRegistryUrl ?? '');
    setRustupDistServer(config.rustupDistServer ?? '');
    setGoproxy(config.goproxy ?? '');
    setHttpProxy(config.httpProxy ?? '');
    setHttpsProxy(config.httpsProxy ?? '');
    setNoProxy(config.noProxy ?? '');
    setJobs(String(config.jobs ?? 4));
    setMinimumReleaseAge(config.minimumReleaseAge ?? '');
    setMiseConfigToml(config.miseConfigToml ?? '');
    setGithubTokenInput('');
  }, [config]);

  if (!config) {
    return (
      <StatusBanner
        tone="info"
        title="未读取到工具配置"
        description="请先启动目标容器，随后即可在线配置 APT / mise / uv 镜像源与 config.toml。"
      />
    );
  }

  const handleSave = async () => {
    const payload: ToolsConfigSetParams = {
      container: config.container,
      mirrorPreset,
      aptMirrorUrl: aptMirrorUrl.trim(),
      nodeMirrorUrl: nodeMirrorUrl.trim(),
      pypiIndexUrl: pypiIndexUrl.trim(),
      npmRegistryUrl: npmRegistryUrl.trim(),
      rustupDistServer: rustupDistServer.trim(),
      goproxy: goproxy.trim(),
      httpProxy: httpProxy.trim(),
      httpsProxy: httpsProxy.trim(),
      noProxy: noProxy.trim(),
      jobs: Math.max(1, Math.min(16, Number(jobs) || 4)),
      minimumReleaseAge: minimumReleaseAge.trim(),
      miseConfigToml,
      ...(githubTokenInput.trim() ? { githubToken: githubTokenInput.trim() } : {}),
    };
    await onSave(payload);
    setGithubTokenInput('');
  };

  return (
    <div className="flex flex-col gap-4">
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="text-sm font-semibold text-[var(--text-primary)]">
            镜像源预设与上游加速
          </span>
          <SegmentedControl
            size="sm"
            value={mirrorPreset}
            onValueChange={setMirrorPreset}
            options={[
              { value: 'tuna', label: '清华 TUNA' },
              { value: 'official', label: '官方上游' },
              { value: 'custom', label: '自定义' },
            ]}
          />
        </div>

        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">APT 镜像源 URL</span>
            <Input mono value={aptMirrorUrl} onChange={(e) => setAptMirrorUrl(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">Node.js 镜像源</span>
            <Input mono value={nodeMirrorUrl} onChange={(e) => setNodeMirrorUrl(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">uv / PyPI 索引 URL</span>
            <Input mono value={pypiIndexUrl} onChange={(e) => setPypiIndexUrl(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">npm Registry</span>
            <Input
              mono
              value={npmRegistryUrl}
              onChange={(e) => setNpmRegistryUrl(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">Rustup Dist Server</span>
            <Input
              mono
              value={rustupDistServer}
              onChange={(e) => setRustupDistServer(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">GOPROXY</span>
            <Input mono value={goproxy} onChange={(e) => setGoproxy(e.target.value)} />
          </div>
        </div>

        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-4">
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">HTTP_PROXY</span>
            <Input mono value={httpProxy} onChange={(e) => setHttpProxy(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">HTTPS_PROXY</span>
            <Input mono value={httpsProxy} onChange={(e) => setHttpsProxy(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">并发任务数 (jobs)</span>
            <Input mono value={jobs} onChange={(e) => setJobs(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">最小发布缓冲期</span>
            <Input
              mono
              value={minimumReleaseAge}
              placeholder="24h"
              onChange={(e) => setMinimumReleaseAge(e.target.value)}
            />
          </div>
        </div>
      </Card>

      {/* Masked MISE_GITHUB_TOKEN */}
      <Card className="flex flex-col gap-2.5 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <KeyRound className="size-4 text-[var(--accent)]" />
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              MISE_GITHUB_TOKEN（脱敏凭据）
            </span>
          </div>
          <Badge variant={config.hasGithubToken ? 'success' : 'neutral'}>
            {config.hasGithubToken ? `已配置 (${config.githubTokenMasked ?? '****'})` : '未配置'}
          </Badge>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            mono
            type="password"
            className="min-w-[240px] flex-1"
            value={githubTokenInput}
            placeholder="输入新 GitHub Token 以更新（服务端永不回显明文）"
            onChange={(e) => setGithubTokenInput(e.target.value)}
          />
          {config.hasGithubToken && (
            <Button
              size="sm"
              variant="danger"
              disabled={busy}
              onClick={() => void onSave({ container: config.container, githubToken: '' })}
            >
              <Trash2 className="size-3.5" />
              清空 Token
            </Button>
          )}
        </div>
      </Card>

      {/* mise config.toml */}
      <Card className="flex flex-col gap-2.5 p-4">
        <div className="flex items-center justify-between gap-2">
          <span className="font-mono text-xs font-semibold text-[var(--text-primary)]">
            /etc/strata/mise/config.toml
          </span>
          <Button size="sm" variant="primary" disabled={busy} onClick={() => void handleSave()}>
            <Save className="size-3.5" />
            保存镜像与配置
          </Button>
        </div>
        <textarea
          rows={9}
          value={miseConfigToml}
          onChange={(e) => setMiseConfigToml(e.target.value)}
          className="w-full rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
        />
      </Card>
    </div>
  );
}
