import { Card, StatusBanner, ThemeSwitcher } from '@gojona/ui';
import { Palette } from 'lucide-react';
import type { WebStatusResult } from '../types.ts';
import { PcPairingCard } from './PcPairingCard.tsx';

/** Props accepted by `KsuSettingsTab`. */
export interface KsuSettingsTabProps {
  webStatus: WebStatusResult | null;
  pairingToken: string | null;
  busy: boolean;
  onToggleLan: (lanEnabled: boolean) => Promise<void>;
  onReadToken: () => Promise<void>;
  onRotateToken: () => Promise<void>;
  onCopyText: (text: string, label: string) => Promise<void>;
}

/** Global daemon-wide settings tab (PC Desktop pairing and UI appearance), independent of active container. */
export function KsuSettingsTab({
  webStatus,
  pairingToken,
  busy,
  onToggleLan,
  onReadToken,
  onRotateToken,
  onCopyText,
}: KsuSettingsTabProps) {
  return (
    <div className="flex flex-col gap-3">
      <StatusBanner
        tone="info"
        title="全局系统设置"
        description="以下设置作用于整个 Strata 后台服务与所有容器，不随上方选中的容器变化。"
      />

      <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
        <div className="md:col-span-6">
          <PcPairingCard
            webStatus={webStatus}
            pairingToken={pairingToken}
            busy={busy}
            onToggleLan={onToggleLan}
            onReadToken={onReadToken}
            onRotateToken={onRotateToken}
            onCopyText={onCopyText}
          />
        </div>

        <div className="md:col-span-6">
          <Card className="flex flex-col gap-2.5 p-3">
            <div className="flex items-center gap-1.5">
              <Palette className="size-3.5 shrink-0 text-[var(--accent)]" />
              <span className="text-xs font-medium text-[var(--text-primary)]">界面外观与主题</span>
            </div>
            <ThemeSwitcher compact />
          </Card>
        </div>
      </div>
    </div>
  );
}
