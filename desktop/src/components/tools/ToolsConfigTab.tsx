import { Badge, Button, Card, Input, SegmentedControl, StatusBanner } from '@gojona/ui';
import { RotateCcw, Save } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import type {
  ToolMirrorPreset,
  ToolsConfigResult,
  ToolsConfigSetParams,
} from '../../types/protocol.ts';
import { MiseSettingsCard, type UrlReplacementRow } from './MiseSettingsCard.tsx';
import { presetMirrorDefaults } from './toolHelpers.ts';

export interface ToolsConfigTabProps {
  config: ToolsConfigResult | null;
  containerRunning: boolean;
  busy: boolean;
  onSelectConfigPath: (path: string) => void;
  onSave: (params: ToolsConfigSetParams) => Promise<void>;
}

/** Mirror preset, proxy (`HTTP/HTTPS/NO_PROXY`), token, and `mise.toml` editor. */
export function ToolsConfigTab({
  config,
  containerRunning,
  busy,
  onSelectConfigPath,
  onSave,
}: ToolsConfigTabProps) {
  const [editMode, setEditMode] = useState<'visual' | 'toml'>('visual');
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
  const [urlReplRows, setUrlReplRows] = useState<UrlReplacementRow[]>([]);
  const [githubTokenInput, setGithubTokenInput] = useState('');
  const [miseConfigToml, setMiseConfigToml] = useState('');
  const [dirty, setDirty] = useState(false);

  const syncFromConfig = useCallback((cfg: ToolsConfigResult) => {
    setMirrorPreset(cfg.mirrorPreset);
    setAptMirrorUrl(cfg.aptMirrorUrl ?? '');
    setNodeMirrorUrl(cfg.nodeMirrorUrl ?? '');
    setPypiIndexUrl(cfg.pypiIndexUrl ?? '');
    setNpmRegistryUrl(cfg.npmRegistryUrl ?? '');
    setRustupDistServer(cfg.rustupDistServer ?? '');
    setGoproxy(cfg.goproxy ?? '');
    setHttpProxy(cfg.httpProxy ?? '');
    setHttpsProxy(cfg.httpsProxy ?? '');
    setNoProxy(cfg.noProxy ?? '');
    setJobs(String(cfg.jobs ?? 4));
    setMinimumReleaseAge(cfg.minimumReleaseAge ?? '');
    setUrlReplRows(
      Object.entries(cfg.urlReplacements ?? {}).map(([from, to], idx) => ({
        uid: `init-${idx}-${from}`,
        from,
        to,
      })),
    );
    setMiseConfigToml(cfg.miseConfigToml ?? '');
    setGithubTokenInput('');
    setDirty(false);
  }, []);

  useEffect(() => {
    if (config) syncFromConfig(config);
  }, [config, syncFromConfig]);

  if (!config) {
    return (
      <StatusBanner
        tone="info"
        title="未读取到工具配置"
        description={
          containerRunning
            ? '正在等待读取容器配置文件…'
            : '请先启动目标容器，随后即可在线配置镜像源、代理与 mise.toml。'
        }
      />
    );
  }

  const handlePresetChange = (next: ToolMirrorPreset) => {
    setMirrorPreset(next);
    const defaults = presetMirrorDefaults(next);
    if (defaults) {
      setAptMirrorUrl(defaults.aptMirrorUrl);
      setNodeMirrorUrl(defaults.nodeMirrorUrl);
      setPypiIndexUrl(defaults.pypiIndexUrl);
      setNpmRegistryUrl(defaults.npmRegistryUrl);
      setRustupDistServer(defaults.rustupDistServer);
      setGoproxy(defaults.goproxy);
    }
    setDirty(true);
  };

  const handleMirrorFieldEdit = (setter: (v: string) => void, val: string) => {
    setter(val);
    if (mirrorPreset !== 'custom') setMirrorPreset('custom');
    setDirty(true);
  };

  const handleSave = async () => {
    const urlReplacements = Object.fromEntries(
      urlReplRows.map((r) => [r.from.trim(), r.to.trim()]).filter(([k, v]) => k && v),
    );
    const payload: ToolsConfigSetParams = {
      container: config.container,
      ...(config.configPath ? { configPath: config.configPath } : {}),
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
      ...(editMode === 'visual'
        ? {
            jobs: Math.max(1, Math.min(32, Number(jobs) || 4)),
            minimumReleaseAge: minimumReleaseAge.trim(),
            urlReplacements,
          }
        : { miseConfigToml }),
      ...(githubTokenInput.trim() ? { githubToken: githubTokenInput.trim() } : {}),
    };
    await onSave(payload);
    setGithubTokenInput('');
    setDirty(false);
  };

  const mirrorFields: Array<[string, string, (v: string) => void]> = [
    ['APT 镜像源 URL', aptMirrorUrl, setAptMirrorUrl],
    ['Node 镜像源 URL', nodeMirrorUrl, setNodeMirrorUrl],
    ['PyPI / uv Index URL', pypiIndexUrl, setPypiIndexUrl],
    ['npm Registry', npmRegistryUrl, setNpmRegistryUrl],
    ['Rustup Dist Server', rustupDistServer, setRustupDistServer],
    ['GOPROXY', goproxy, setGoproxy],
  ];

  return (
    <div className="flex flex-col gap-3">
      <Card className="sticky top-0 z-10 flex flex-wrap items-center justify-between gap-2 border-[var(--border-hairline)] bg-[var(--surface-elevated)]/95 px-4 py-2.5 backdrop-blur-xs">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs font-semibold text-[var(--text-primary)]">配置编辑</span>
          {dirty ? (
            <Badge variant="warning">有未保存变更</Badge>
          ) : (
            <Badge variant="success">已同步</Badge>
          )}
          {config.configPaths.length > 0 && (
            <div className="flex flex-wrap items-center gap-1">
              {config.configPaths.map((p) => (
                <Button
                  key={p}
                  size="sm"
                  variant={config.configPath === p ? 'pill' : 'ghost'}
                  onClick={() => onSelectConfigPath(p)}
                >
                  {p}
                </Button>
              ))}
            </div>
          )}
        </div>
        <div className="flex items-center gap-2">
          {dirty && (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => syncFromConfig(config)}
            >
              <RotateCcw className="size-3.5" />
              放弃修改
            </Button>
          )}
          <Button
            size="sm"
            variant="primary"
            disabled={busy || !containerRunning}
            onClick={() => void handleSave()}
          >
            <Save className="size-3.5" />
            保存配置
          </Button>
        </div>
      </Card>

      <Card className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="text-sm font-semibold text-[var(--text-primary)]">
            镜像源预设与上游加速
          </span>
          <SegmentedControl
            size="sm"
            value={mirrorPreset}
            onValueChange={handlePresetChange}
            options={[
              { value: 'tuna', label: '清华 TUNA' },
              { value: 'official', label: '官方上游' },
              { value: 'custom', label: '自定义' },
            ]}
          />
        </div>
        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
          {mirrorFields.map(([label, val, setter]) => (
            <div key={label} className="flex flex-col gap-1">
              <span className="text-xs text-[var(--text-secondary)]">{label}</span>
              <Input
                mono
                value={val}
                placeholder={mirrorPreset === 'official' ? '使用官方默认上游' : ''}
                onChange={(e) => handleMirrorFieldEdit(setter, e.target.value)}
              />
            </div>
          ))}
        </div>
        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-3">
          {[
            ['HTTP_PROXY', httpProxy, setHttpProxy, ''],
            ['HTTPS_PROXY', httpsProxy, setHttpsProxy, ''],
            ['NO_PROXY', noProxy, setNoProxy, 'localhost,127.0.0.1'],
          ].map(([label, val, setter, ph]) => (
            <div key={String(label)} className="flex flex-col gap-1">
              <span className="text-xs text-[var(--text-secondary)]">{String(label)}</span>
              <Input
                mono
                value={String(val)}
                placeholder={String(ph)}
                onChange={(e) => {
                  (setter as (v: string) => void)(e.target.value);
                  setDirty(true);
                }}
              />
            </div>
          ))}
        </div>
      </Card>

      <MiseSettingsCard
        configPathLabel={config.configPath ?? '/root/.config/mise/config.toml'}
        editMode={editMode}
        jobs={jobs}
        minimumReleaseAge={minimumReleaseAge}
        urlReplRows={urlReplRows}
        miseConfigToml={miseConfigToml}
        hasGithubToken={config.hasGithubToken}
        {...(config.githubTokenMasked ? { githubTokenMasked: config.githubTokenMasked } : {})}
        githubTokenInput={githubTokenInput}
        containerRunning={containerRunning}
        busy={busy}
        onEditModeChange={setEditMode}
        onJobsChange={(v) => {
          setJobs(v);
          setDirty(true);
        }}
        onMinimumReleaseAgeChange={(v) => {
          setMinimumReleaseAge(v);
          setDirty(true);
        }}
        onUrlReplRowsChange={(rows) => {
          setUrlReplRows(rows);
          setDirty(true);
        }}
        onTomlChange={(v) => {
          setMiseConfigToml(v);
          setDirty(true);
        }}
        onGithubTokenInputChange={(v) => {
          setGithubTokenInput(v);
          setDirty(true);
        }}
        onClearGithubToken={() => void onSave({ container: config.container, githubToken: '' })}
      />
    </div>
  );
}
