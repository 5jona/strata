import { Badge, Button, Card } from '@gojona/ui';
import { Download, Layers, Sparkles, Terminal } from 'lucide-react';
import type { ToolBundleSpec, ToolManagerKind } from '../../types/protocol.ts';
import { SHELL_PRESET_META } from './toolHelpers.ts';

export interface ToolsBundlePresetsProps {
  containerId: string;
  bundles: ToolBundleSpec[];
  disabled: boolean;
  onInstallBundle: (manager: ToolManagerKind, packages: string[], pin: boolean) => void;
  onOpenTerminal?: (containerId: string) => void;
}

/** Curated modern Shell presets and ARM64 dev/debug tool bundles. */
export function ToolsBundlePresets({
  containerId,
  bundles,
  disabled,
  onInstallBundle,
  onOpenTerminal,
}: ToolsBundlePresetsProps) {
  const shellBundles = bundles.filter((b) => b.id.startsWith('shell-'));
  const devBundles = bundles.filter((b) => !b.id.startsWith('shell-'));

  return (
    <div className="flex flex-col gap-4">
      {shellBundles.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
              <Sparkles className="size-3.5 text-[var(--accent)]" />
              <span>现代终端体验一键预设（Shell + Starship + zoxide + eza + fzf）</span>
            </div>
            {onOpenTerminal && (
              <Button size="sm" variant="ghost" onClick={() => onOpenTerminal(containerId)}>
                <Terminal className="size-3.5" />
                去终端体验
              </Button>
            )}
          </div>
          <div className="grid grid-cols-1 gap-2.5 md:grid-cols-3">
            {shellBundles.map((bundle) => {
              const meta = SHELL_PRESET_META[bundle.id];
              return (
                <Card key={bundle.id} className="flex flex-col justify-between gap-2.5 p-3.5">
                  <div className="flex flex-col gap-1.5">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm font-semibold text-[var(--text-primary)]">
                        {bundle.name}
                      </span>
                      {meta && <Badge variant={meta.tagVariant}>{meta.tag}</Badge>}
                    </div>
                    <p className="text-xs text-[var(--text-secondary)]">
                      {meta?.note ?? bundle.summary}
                    </p>
                    <div className="flex flex-wrap gap-1">
                      {bundle.tools.map((t) => (
                        <Badge key={t} variant="neutral">
                          {t.replace(/^(apt|mise|uv):/, '')}
                        </Badge>
                      ))}
                    </div>
                  </div>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={disabled}
                    onClick={() => onInstallBundle('bundle', [bundle.id], true)}
                  >
                    <Download className="size-3.5" />
                    一键安装并配置
                  </Button>
                </Card>
              );
            })}
          </div>
        </div>
      )}

      {devBundles.length > 0 && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
            <Layers className="size-3.5 text-[var(--accent)]" />
            <span>ARM64 常用开发与诊断套件</span>
          </div>
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-4">
            {devBundles.map((bundle) => (
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
                  disabled={disabled}
                  onClick={() => onInstallBundle('bundle', [bundle.id], true)}
                >
                  <Download className="size-3.5" />
                  一键安装
                </Button>
              </Card>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
