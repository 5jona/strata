import { Badge, Button, Card, Input } from '@gojona/ui';
import { KeyRound, Plus } from 'lucide-react';
import { useState } from 'react';
import { summarizeSshKey } from '../lib/strataFormat.ts';

/** Props accepted by `SshKeysCard`. */
export interface SshKeysCardProps {
  sshKeys: readonly string[];
  busy: boolean;
  onAddSshKey: (publicKey: string) => Promise<boolean>;
  onImeFocusChange: (focused: boolean) => void;
}

/** Card listing container SSH authorized keys and providing an input to append new public keys. */
export function SshKeysCard({ sshKeys, busy, onAddSshKey, onImeFocusChange }: SshKeysCardProps) {
  const [newKey, setNewKey] = useState('');

  const handleAddKey = async () => {
    const trimmed = newKey.trim();
    if (!trimmed) return;
    const ok = await onAddSshKey(trimmed);
    if (ok) setNewKey('');
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
            尚未配置公钥（Dropbear 仅允许公钥登录）。
          </p>
        ) : (
          sshKeys.map((line) => {
            const summary = summarizeSshKey(line);
            return (
              <div
                key={line}
                className="flex items-center justify-between gap-2 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5"
              >
                <span className="truncate font-mono text-[11px] text-[var(--text-primary)]">
                  {summary.label}
                </span>
                <Badge className="shrink-0" variant="accent">
                  {summary.algo}
                </Badge>
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
          onFocus={() => onImeFocusChange(true)}
          onBlur={() => onImeFocusChange(false)}
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
