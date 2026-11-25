import { Badge, Button, Card, SettingRow, Switch } from '@gojona/ui';
import { Copy, KeyRound, Laptop, RefreshCw } from 'lucide-react';
import { STRATA_PATHS, STRATA_PROTOCOL, type WebStatusResult } from '../types.ts';

/** Props accepted by `PcPairingCard`. */
export interface PcPairingCardProps {
  webStatus: WebStatusResult | null;
  pairingToken: string | null;
  busy: boolean;
  onToggleLan: (lanEnabled: boolean) => Promise<void>;
  onRotateToken: () => Promise<void>;
  onReadToken?: () => Promise<void>;
  onCopyText: (text: string, label: string) => Promise<void>;
}

/**
 * Compact PC Desktop pairing card (`web.status`, LAN switch, address & Bearer token copy/rotate).
 */
export function PcPairingCard({
  webStatus,
  pairingToken,
  busy,
  onToggleLan,
  onRotateToken,
  onReadToken,
  onCopyText,
}: PcPairingCardProps) {
  const endpointUrl =
    webStatus?.lanEnabled && webStatus.lanUrls[0]
      ? webStatus.lanUrls[0]
      : `http://127.0.0.1:${webStatus?.port ?? STRATA_PROTOCOL.DEFAULT_WEB_PORT}`;

  return (
    <Card className="flex flex-col gap-2.5 p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <Laptop className="size-3.5 shrink-0 text-[var(--accent)]" />
          <span className="truncate text-xs font-medium text-[var(--text-primary)]">
            PC 配对 / 远程控制
          </span>
        </div>
        <Badge
          className="shrink-0"
          variant={webStatus?.running ? (webStatus.lanEnabled ? 'warning' : 'success') : 'neutral'}
        >
          {webStatus?.running
            ? webStatus.lanEnabled
              ? `LAN :${webStatus.port}`
              : `本机 :${webStatus.port}`
            : '未监听'}
        </Badge>
      </div>

      <SettingRow
        variant="plain"
        title="允许局域网 (RFC1918) 配对"
        description="开启后电脑端 Strata Desktop 可直接通过 Wi-Fi + Token 管理"
        control={
          <Switch
            checked={Boolean(webStatus?.lanEnabled)}
            disabled={busy || !webStatus}
            aria-label="允许局域网配对"
            onCheckedChange={(checked) => void onToggleLan(checked)}
          />
        }
      />

      <div className="flex items-center justify-between gap-1.5 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5">
        <span className="truncate font-mono text-[11px] text-[var(--text-primary)]">
          {endpointUrl}
        </span>
        <Button
          className="shrink-0"
          size="sm"
          variant="ghost"
          aria-label="复制控制面地址"
          onClick={() => void onCopyText(endpointUrl, '控制面地址')}
        >
          <Copy className="size-3" />
          复制
        </Button>
      </div>

      {pairingToken ? (
        <div className="flex items-center justify-between gap-1.5 rounded-[var(--radius-control)] border border-[color-mix(in_oklab,var(--accent)_35%,transparent)] bg-[var(--accent-soft)] px-2.5 py-1.5">
          <span className="truncate font-mono text-[10px] text-[var(--text-primary)]">
            Token: {`${pairingToken.slice(0, 10)}…${pairingToken.slice(-6)}`}
          </span>
          <Button
            className="shrink-0"
            size="sm"
            variant="pill"
            aria-label="复制 Bearer Token"
            onClick={() => void onCopyText(pairingToken, 'Bearer Token')}
          >
            <Copy className="size-3" />
            复制 Token
          </Button>
        </div>
      ) : null}

      <div className="flex flex-wrap items-center justify-between gap-1.5">
        {onReadToken ? (
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            aria-label="读取当前 Bearer Token"
            onClick={() => void onReadToken()}
          >
            <KeyRound className="size-3.5" />
            读取当前 Token
          </Button>
        ) : (
          <span className="truncate font-mono text-[10px] text-[var(--text-secondary)]">
            {webStatus?.tokenPath ?? STRATA_PATHS.DEFAULT_WEB_TOKEN_PATH}
          </span>
        )}
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || !webStatus}
          aria-label="轮换并复制新 Bearer Token"
          onClick={() => void onRotateToken()}
        >
          <RefreshCw className="size-3.5" />
          轮换 Token
        </Button>
      </div>
    </Card>
  );
}
