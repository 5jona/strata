import { Badge, Button, Card, StatusBanner } from '@gojona/ui';
import { AlertTriangle, CheckCircle2, RefreshCw, Stethoscope, XCircle } from 'lucide-react';
import { useState } from 'react';
import type { ToolDiagnosticStatus, ToolsDoctorResult } from '../../types/protocol.ts';

function statusMeta(status: ToolDiagnosticStatus): {
  label: string;
  variant: 'success' | 'warning' | 'danger';
} {
  switch (status) {
    case 'ok':
      return { label: '通过', variant: 'success' };
    case 'warn':
      return { label: '警告', variant: 'warning' };
    case 'error':
      return { label: '异常', variant: 'danger' };
  }
}

export interface ToolsDoctorTabProps {
  doctor: ToolsDoctorResult | null;
  busy: boolean;
  onRunDoctor: () => void;
}

/**
 * Structured container layout, PATH, XDG, APT sandbox, and Dropbear diagnostics (`tools.doctor`).
 */
export function ToolsDoctorTab({ doctor, busy, onRunDoctor }: ToolsDoctorTabProps) {
  const [showRawMise, setShowRawMise] = useState(false);

  if (!doctor) {
    return (
      <StatusBanner
        tone="info"
        title="尚未运行环境体检"
        description="点击下方刷新按钮检查 4 路径标准布局、PATH/XDG、APT 沙盒及 Dropbear 状态。"
        action={
          <Button size="sm" variant="primary" disabled={busy} onClick={onRunDoctor}>
            <Stethoscope className="size-3.5" />
            立即体检
          </Button>
        }
      />
    );
  }

  const warnCount = doctor.items.filter((i) => i.status === 'warn').length;
  const errCount = doctor.items.filter((i) => i.status === 'error').length;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <StatusBanner
          className="flex-1"
          tone={doctor.ok && warnCount === 0 ? 'success' : errCount > 0 ? 'danger' : 'warning'}
          title={
            doctor.ok
              ? warnCount === 0
                ? '容器工具环境体检全部通过'
                : `容器基础可用（${warnCount} 项警告）`
              : `检测到 ${errCount} 项异常，需要修复`
          }
          description={`共检测 ${doctor.items.length} 项（容器 ${doctor.container}）`}
        />
        <Button size="sm" variant="secondary" disabled={busy} onClick={onRunDoctor}>
          <RefreshCw className={`size-3.5 ${busy ? 'animate-spin' : ''}`} />
          重新体检
        </Button>
      </div>

      <div className="flex flex-col gap-2">
        {doctor.items.map((item) => {
          const meta = statusMeta(item.status);
          return (
            <Card key={item.id} className="flex flex-col gap-1.5 p-3.5">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  {item.status === 'ok' ? (
                    <CheckCircle2 className="size-4 text-[var(--success)]" />
                  ) : item.status === 'warn' ? (
                    <AlertTriangle className="size-4 text-[var(--warning)]" />
                  ) : (
                    <XCircle className="size-4 text-[var(--danger)]" />
                  )}
                  <span className="font-mono text-xs font-semibold text-[var(--text-primary)]">
                    {item.id}
                  </span>
                  <Badge variant="neutral">{item.category}</Badge>
                </div>
                <Badge variant={meta.variant}>{meta.label}</Badge>
              </div>
              <p className="text-xs text-[var(--text-primary)]">{item.summary}</p>
              {item.remediation && (
                <div className="rounded-md border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5 font-mono text-[11px] text-[var(--text-secondary)]">
                  修复指引：{item.remediation}
                </div>
              )}
            </Card>
          );
        })}
      </div>

      {doctor.miseDoctorJson && (
        <Card className="flex flex-col gap-2 p-3.5">
          <div className="flex items-center justify-between">
            <span className="font-mono text-xs font-semibold text-[var(--text-primary)]">
              mise doctor --json
            </span>
            <Button size="sm" variant="ghost" onClick={() => setShowRawMise((v) => !v)}>
              {showRawMise ? '收起 JSON' : '展开 JSON'}
            </Button>
          </div>
          {showRawMise && (
            <pre className="max-h-56 overflow-auto rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-2.5 font-mono text-[11px] text-[var(--text-primary)]">
              {doctor.miseDoctorJson}
            </pre>
          )}
        </Card>
      )}
    </div>
  );
}
