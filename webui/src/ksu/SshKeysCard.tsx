import { Badge, Button, Card, Input } from '@gojona/ui';
import { KeyRound, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { summarizeSshKey } from '../shared/strataFormat.ts';

/** Props accepted by `SshKeysCard`. */
export interface SshKeysCardProps {
  sshKeys: readonly string[];
  busy: boolean;
  onAddSshKey: (publicKey: string) => Promise<boolean>;
  onRemoveSshKey: (fingerprintOrKey: string) => Promise<boolean>;
}

/** Card listing container SSH authorized keys with add and delete actions for KSU management. */
export function SshKeysCard({ sshKeys, busy, onAddSshKey, onRemoveSshKey }: SshKeysCardProps) {
  const [newKey, setNewKey] = useState('');
  const [confirmKey, setConfirmKey] = useState<string | null>(null);

  const handleAddKey = async () => {
    const trimmed = newKey.trim();
    if (!trimmed) return;
    const ok = await onAddSshKey(trimmed);
    if (ok) setNewKey('');
  };

  const handleConfirmRemove = async (keyLine: string) => {
    const ok = await onRemoveSshKey(keyLine);
    if (ok) setConfirmKey(null);
  };

  return (
    <Card className="flex flex-col gap-2.5 p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <KeyRound className="size-3.5 shrink-0 text-[var(--accent)]" />
          <span className="truncate text-xs font-medium text-[var(--text-primary)]">
            SSH 授权公钥
          </span>
        </div>
        <Badge className="shrink-0" variant="neutral">
          {sshKeys.length} 条
        </Badge>
      </div>

      <div className="flex flex-col gap-1.5">
        {sshKeys.length === 0 ? (
          <p className="text-[11px] text-[var(--text-secondary)]">
            尚未配置公钥（SSH 服务仅允许公钥免密登录）。
          </p>
        ) : (
          sshKeys.map((line) => {
            const summary = summarizeSshKey(line);
            const isConfirming = confirmKey === line;
            return (
              <div
                key={line}
                className="flex items-center justify-between gap-2 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5"
              >
                <div className="flex min-w-0 flex-1 items-center gap-1.5">
                  <Badge className="shrink-0" variant="accent">
                    {summary.algo}
                  </Badge>
                  <span className="truncate font-mono text-[11px] text-[var(--text-primary)]">
                    {summary.label}
                  </span>
                </div>

                {isConfirming ? (
                  <div className="flex shrink-0 items-center gap-1">
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={busy}
                      aria-label={`确认删除公钥 ${summary.label}`}
                      onClick={() => void handleConfirmRemove(line)}
                    >
                      删除
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => setConfirmKey(null)}
                    >
                      取消
                    </Button>
                  </div>
                ) : (
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    disabled={busy}
                    aria-label={`删除公钥 ${summary.label}`}
                    onClick={() => setConfirmKey(line)}
                  >
                    <Trash2 className="size-3.5 text-[var(--danger)]" />
                  </Button>
                )}
              </div>
            );
          })
        )}
      </div>

      <div className="flex items-center gap-1.5">
        <Input
          mono
          value={newKey}
          placeholder="粘贴 ssh-ed25519 / ssh-rsa 公钥…"
          aria-label="新增 SSH 公钥输入框"
          onChange={(e) => setNewKey(e.target.value)}
        />
        <Button
          className="shrink-0"
          size="sm"
          variant="secondary"
          disabled={busy || newKey.trim().length === 0}
          aria-label="追加 SSH 公钥"
          onClick={() => void handleAddKey()}
        >
          <Plus className="size-3.5" />
          添加
        </Button>
      </div>
    </Card>
  );
}
