import {
  Button,
  Card,
  Input,
  SegmentedControl,
  SettingRow,
  StatusBanner,
  Switch,
} from '@gojona/ui';
import { X } from 'lucide-react';
import { useEffect, useState } from 'react';
import {
  formatEnvLines,
  parseCommandArgv,
  parseEnvLines,
  validateServiceSpec,
} from '../lib/format.ts';
import type { ContainerServiceSpec, ServiceRestartPolicy } from '../types/protocol.ts';

export interface ServiceModalProps {
  open: boolean;
  containerId: string;
  initialSpec: ContainerServiceSpec | null;
  busy: boolean;
  onClose: () => void;
  onSubmit: (spec: ContainerServiceSpec) => Promise<void>;
}

/**
 * Form modal for creating or editing a managed auto-starting container background service.
 */
export function ServiceModal({
  open,
  containerId,
  initialSpec,
  busy,
  onClose,
  onSubmit,
}: ServiceModalProps) {
  const [id, setId] = useState('');
  const [name, setName] = useState('');
  const [command, setCommand] = useState('');
  const [workdir, setWorkdir] = useState('/root');
  const [envText, setEnvText] = useState('');
  const [autoStart, setAutoStart] = useState(true);
  const [restartPolicy, setRestartPolicy] = useState<ServiceRestartPolicy>('on-failure');
  const [maxRestarts, setMaxRestarts] = useState('5');
  const [backoffMs, setBackoffMs] = useState('2000');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setError(null);
    if (initialSpec) {
      setId(initialSpec.id);
      setName(initialSpec.name);
      setCommand(initialSpec.command.join(' '));
      setWorkdir(initialSpec.workdir ?? '/root');
      setEnvText(formatEnvLines(initialSpec.env));
      setAutoStart(initialSpec.autoStart);
      setRestartPolicy(initialSpec.restartPolicy);
      setMaxRestarts(String(initialSpec.maxRestarts));
      setBackoffMs(String(initialSpec.backoffMs));
    } else {
      setId('');
      setName('');
      setCommand('');
      setWorkdir('/root');
      setEnvText('');
      setAutoStart(true);
      setRestartPolicy('on-failure');
      setMaxRestarts('5');
      setBackoffMs('2000');
    }
  }, [initialSpec, open]);

  if (!open) return null;

  const handleSave = async () => {
    const spec: ContainerServiceSpec = {
      id: id.trim(),
      name: name.trim(),
      command: parseCommandArgv(command),
      ...(workdir.trim() ? { workdir: workdir.trim() } : {}),
      env: parseEnvLines(envText),
      autoStart,
      restartPolicy,
      maxRestarts: Number(maxRestarts) || 0,
      backoffMs: Number(backoffMs) || 2000,
    };
    const validationErr = validateServiceSpec(spec);
    if (validationErr) {
      setError(validationErr);
      return;
    }
    await onSubmit(spec);
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
    >
      <Card className="flex w-full max-w-lg flex-col gap-3.5 p-5">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              {initialSpec ? `编辑托管服务 · ${initialSpec.id}` : `新增托管服务 · ${containerId}`}
            </h3>
            <p className="text-xs text-[var(--text-secondary)]">
              由 stratad 原生守护，支持开机静默自启、崩溃退避重启与独立日志采集
            </p>
          </div>
          <Button size="icon-sm" variant="ghost" aria-label="关闭" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>

        {error && <StatusBanner tone="danger" title={error} />}

        <div className="grid grid-cols-2 gap-2.5">
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">服务 ID</span>
            <Input
              mono
              disabled={Boolean(initialSpec)}
              value={id}
              placeholder="例如：rsshub"
              onChange={(e) => setId(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">显示名称</span>
            <Input
              value={name}
              placeholder="例如：RSSHub 订阅服务"
              onChange={(e) => setName(e.target.value)}
            />
          </div>
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-xs text-[var(--text-secondary)]">启动命令（argv 空格分隔）</span>
          <Input
            mono
            value={command}
            placeholder="node /opt/rsshub/dist/index.mjs"
            onChange={(e) => setCommand(e.target.value)}
          />
        </div>

        <div className="grid grid-cols-3 gap-2.5">
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">工作目录</span>
            <Input mono value={workdir} onChange={(e) => setWorkdir(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">最大重启次数</span>
            <Input mono value={maxRestarts} onChange={(e) => setMaxRestarts(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">退避间隔 (ms)</span>
            <Input mono value={backoffMs} onChange={(e) => setBackoffMs(e.target.value)} />
          </div>
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-xs text-[var(--text-secondary)]">重启策略</span>
          <SegmentedControl
            value={restartPolicy}
            onValueChange={setRestartPolicy}
            size="sm"
            fullWidth
            options={[
              { value: 'on-failure', label: '失败时重启' },
              { value: 'always', label: '始终重启' },
              { value: 'never', label: '从不重启' },
            ]}
          />
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-xs text-[var(--text-secondary)]">环境变量（每行 KEY=VALUE）</span>
          <textarea
            rows={3}
            value={envText}
            onChange={(e) => setEnvText(e.target.value)}
            placeholder={'PORT=1200\nNODE_ENV=production'}
            className="w-full rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-2 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
          />
        </div>

        <SettingRow
          variant="plain"
          title="随容器开机静默自启"
          description="容器启动后立即自动拉起该后台服务"
          control={<Switch checked={autoStart} onCheckedChange={setAutoStart} />}
        />

        <div className="flex justify-end gap-2 pt-1">
          <Button size="sm" variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button size="sm" variant="primary" disabled={busy} onClick={() => void handleSave()}>
            保存服务
          </Button>
        </div>
      </Card>
    </div>
  );
}
