import { Button, Card } from '@gojona/ui';
import { Plus, RefreshCw, Smartphone } from 'lucide-react';

export interface FirstRunOnboardingProps {
  scanningAdb: boolean;
  onOpenAdd: () => void;
  onScanAdb: () => void;
}

/** First-run onboarding card shown when no Android device is selected or paired yet. */
export function FirstRunOnboarding({ scanningAdb, onOpenAdd, onScanAdb }: FirstRunOnboardingProps) {
  return (
    <div className="flex flex-1 items-center justify-center p-8">
      <Card className="flex w-full max-w-lg flex-col gap-4 p-6">
        <div className="flex items-center gap-3">
          <div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-[var(--accent)]/15 text-[var(--accent)]">
            <Smartphone className="size-6" />
          </div>
          <div>
            <h2 className="text-base font-semibold text-[var(--text-primary)]">
              欢迎使用 Strata Desktop
            </h2>
            <p className="text-xs text-[var(--text-secondary)]">
              按以下两步连接 Android 设备，即可管理 Linux 容器、服务与调试环境
            </p>
          </div>
        </div>
        <ol className="flex flex-col gap-2 rounded-xl border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3.5 text-xs leading-relaxed text-[var(--text-primary)]">
          <li>
            <span className="font-semibold text-[var(--accent)]">1. 准备手机端：</span>
            在手机 KernelSU / Magisk 中安装并激活 Strata 模块。
          </li>
          <li>
            <span className="font-semibold text-[var(--accent)]">2. 建立连接：</span>
            通过 USB 线连接电脑扫描 ADB；或打开手机 KernelSU 的 Strata「系统设置 →
            电脑端配对连接」，开启局域网配对并复制配对密钥 (Token)。
          </li>
        </ol>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" onClick={onOpenAdd}>
            <Plus className="size-4" />
            扫描 ADB / 添加设备
          </Button>
          <Button variant="secondary" disabled={scanningAdb} onClick={onScanAdb}>
            <RefreshCw className={`size-3.5 ${scanningAdb ? 'animate-spin' : ''}`} />
            重新扫描 ADB
          </Button>
        </div>
      </Card>
    </div>
  );
}
