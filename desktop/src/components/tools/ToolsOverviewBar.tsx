import { Badge, Button, Card, SegmentedControl, StatusBanner } from '@gojona/ui';
import { Play, RefreshCw, Sparkles, Trash2, Wrench } from 'lucide-react';
import { useState } from 'react';
import { formatBytes } from '../../lib/format.ts';
import type {
  ContainerStatusSnapshot,
  ToolManagerHealth,
  ToolMirrorPreset,
  ToolsBootstrapProfile,
  ToolsStatusResult,
} from '../../types/protocol.ts';
import { toolSourceMeta } from './toolHelpers.ts';

function HealthChip({ label, health }: { label: string; health?: ToolManagerHealth }) {
  const srcMeta = health?.installed ? toolSourceMeta(health.source) : null;
  return (
    <div
      className="flex items-center gap-1.5 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1 text-xs"
      title={health?.binPath ?? '未检测到可执行文件'}
    >
      <span className="font-medium text-[var(--text-primary)]">{label}</span>
      <Badge variant={health?.installed ? 'success' : 'neutral'}>
        {health?.installed ? health.version || '已就绪' : '未安装'}
      </Badge>
      {srcMeta && <Badge variant={srcMeta.variant}>{srcMeta.label}</Badge>}
      {health?.binPath && (
        <span className="hidden max-w-[160px] truncate font-mono text-[10px] text-[var(--text-secondary)] xl:inline">
          {health.binPath}
        </span>
      )}
    </div>
  );
}

export interface ToolsOverviewBarProps {
  containers: ContainerStatusSnapshot[];
  selectedContainer: string;
  activeContainerRunning: boolean;
  status: ToolsStatusResult | null;
  busy: boolean;
  disabled: boolean;
  onSelectContainer: (id: string) => void;
  onStartContainer: () => void;
  onBootstrap: (profile: ToolsBootstrapProfile, mirrorPreset: ToolMirrorPreset) => void;
  onCleanCache: () => void;
  onRefresh: () => void;
}

/** Top overview bar with live manager provenance badges and non-intrusive bootstrap guide. */
export function ToolsOverviewBar({
  containers,
  selectedContainer,
  activeContainerRunning,
  status,
  busy,
  disabled,
  onSelectContainer,
  onStartContainer,
  onBootstrap,
  onCleanCache,
  onRefresh,
}: ToolsOverviewBarProps) {
  const [guideOpen, setGuideOpen] = useState(false);
  const [profile, setProfile] = useState<ToolsBootstrapProfile>('dev');
  const [mirrorPreset, setMirrorPreset] = useState<ToolMirrorPreset>('tuna');

  const needsSetup = Boolean(status && (!status.mise.installed || !status.uv.installed));
  const pkgLabel = status?.systemPkgManager ? `包管理 (${status.systemPkgManager})` : '系统包管理';

  return (
    <Card className="flex shrink-0 flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <Wrench className="size-4 text-[var(--accent)]" />
          <h1 className="text-base font-semibold text-[var(--text-primary)]">容器工具环境</h1>
          <div className="flex items-center gap-1 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-0.5">
            {containers.map((c) => (
              <Button
                key={c.id}
                size="sm"
                variant={selectedContainer === c.id ? 'pill' : 'ghost'}
                onClick={() => onSelectContainer(c.id)}
              >
                <span
                  className={`size-1.5 rounded-full ${
                    c.phase === 'running' ? 'bg-[var(--success)]' : 'bg-[var(--text-secondary)]/50'
                  }`}
                />
                {c.id}
              </Button>
            ))}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant={needsSetup ? 'primary' : 'secondary'}
            disabled={busy || disabled || !activeContainerRunning}
            onClick={() => setGuideOpen((v) => !v)}
          >
            <Sparkles className="size-3.5" />
            {needsSetup ? '安装工具链引导' : '环境补全引导'}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || disabled || !activeContainerRunning}
            onClick={onCleanCache}
          >
            <Trash2 className="size-3.5" />
            清理缓存
          </Button>
          <Button size="sm" variant="ghost" disabled={busy || disabled} onClick={onRefresh}>
            <RefreshCw className={`size-3.5 ${busy ? 'animate-spin' : ''}`} />
            刷新自动识别
          </Button>
        </div>
      </div>

      {!activeContainerRunning && (
        <StatusBanner
          tone="warning"
          title={`容器「${selectedContainer}」当前未运行`}
          description="工具安装、版本切换、源配置与自动探测需要容器处于运行状态。"
          action={
            <Button
              size="sm"
              variant="primary"
              disabled={busy || disabled}
              onClick={onStartContainer}
            >
              <Play className="size-3.5" />
              一键启动容器
            </Button>
          }
        />
      )}

      <div className="flex flex-wrap items-center gap-1.5">
        <HealthChip label="mise" health={status?.mise} />
        <HealthChip label="uv" health={status?.uv} />
        <HealthChip label={pkgLabel} health={status?.apt} />
        <HealthChip label="host-exec" health={status?.hostExec} />
        <HealthChip label="strata-debug" health={status?.strataDebug} />
        {status && (
          <Badge variant="neutral">
            缓存 {formatBytes(status.cacheBytes)} · 已装 {formatBytes(status.installsBytes)}
          </Badge>
        )}
      </div>

      {status?.legacyStrataMiseDetected && (
        <StatusBanner
          tone="info"
          title="检测到 /opt/strata/mise 中的历史工具安装目录"
          description="已自动兼容合并展示旧版托管工具与当前 ~/.local/share/mise 自装工具，无需手工迁移即可继续管理。"
        />
      )}

      {(guideOpen || (activeContainerRunning && needsSetup)) && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3">
          <div className="flex flex-col gap-1">
            <span className="text-xs font-semibold text-[var(--text-primary)]">
              选择初始化方案与镜像源（将优先安装至 ~/.local/bin，不强制覆盖已有配置）
            </span>
            <span className="text-[11px] text-[var(--text-secondary)]">
              Minimal：基础 git/curl · Dev：Python 3.12 + Node LTS + 常用 CLI ·
              Debug：附加系统诊断包
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <SegmentedControl
              size="sm"
              value={mirrorPreset}
              onValueChange={setMirrorPreset}
              options={[
                { value: 'tuna', label: '清华 TUNA' },
                { value: 'official', label: '官方上游' },
              ]}
            />
            <SegmentedControl
              size="sm"
              value={profile}
              onValueChange={setProfile}
              options={[
                { value: 'minimal', label: '基础环境' },
                { value: 'dev', label: '开发套件' },
                { value: 'debug', label: '诊断增强' },
              ]}
            />
            <Button
              size="sm"
              variant="primary"
              disabled={busy || disabled || !activeContainerRunning}
              onClick={() => {
                onBootstrap(profile, mirrorPreset);
                setGuideOpen(false);
              }}
            >
              开始安装
            </Button>
          </div>
        </div>
      )}
    </Card>
  );
}
