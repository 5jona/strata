import { Badge, Button, Card, ProgressBar, StatusBanner } from '@gojona/ui';
import { ArrowRight, RotateCcw, Terminal, Wrench, X, XCircle } from 'lucide-react';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type { ContainerJobProgressPayload } from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';
import { JOB_STAGE_LABELS } from './install/installHelpers.ts';

export interface ProvisionJobsBannerProps {
  transport: StrataTransport | null;
  jobs: ContainerJobProgressPayload[];
  onDismissJob?: (jobId: string) => void;
  onRetryJob?: (job: ContainerJobProgressPayload) => void;
  onOpenTerminal?: (containerId: string) => void;
  onOpenTools?: (containerId: string) => void;
  onBackToContainers?: () => void;
}

/**
 * Displays live provisioning progress, explicit errors, retry/dismiss actions, and post-install next steps.
 */
export function ProvisionJobsBanner({
  transport,
  jobs,
  onDismissJob,
  onRetryJob,
  onOpenTerminal,
  onOpenTools,
  onBackToContainers,
}: ProvisionJobsBannerProps) {
  if (jobs.length === 0) return null;

  return (
    <div className="flex flex-col gap-2">
      {jobs.map((job) => {
        const stageText = JOB_STAGE_LABELS[job.stage] ?? job.stage;
        const badgeVariant =
          job.state === 'completed'
            ? 'success'
            : job.state === 'failed'
              ? 'danger'
              : job.state === 'cancelled'
                ? 'warning'
                : 'accent';

        return (
          <Card key={job.jobId} className="flex flex-col gap-2.5 p-3.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold text-[var(--text-primary)]">
                  {job.container}
                </span>
                <Badge variant={badgeVariant}>{stageText}</Badge>
              </div>
              <div className="flex items-center gap-1.5">
                {job.state === 'running' && transport && !job.jobId.startsWith('local-upload-') && (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      void transport.invoke(STRATA_METHODS.CONTAINER_JOB_CANCEL, {
                        jobId: job.jobId,
                      })
                    }
                  >
                    <XCircle className="size-3.5" />
                    取消
                  </Button>
                )}
                {(job.state === 'failed' || job.state === 'cancelled') && onRetryJob && (
                  <Button size="sm" variant="secondary" onClick={() => onRetryJob(job)}>
                    <RotateCcw className="size-3.5" />
                    重试
                  </Button>
                )}
                {job.state !== 'running' && onDismissJob && (
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label="关闭任务卡片"
                    onClick={() => onDismissJob(job.jobId)}
                  >
                    <X className="size-3.5" />
                  </Button>
                )}
              </div>
            </div>

            <ProgressBar
              value={job.percent ?? 0}
              showValue
              label={job.message ?? '处理中…'}
              tone={
                job.state === 'failed' ? 'danger' : job.state === 'completed' ? 'success' : 'accent'
              }
            />

            {job.error && (
              <StatusBanner tone="danger" title="任务失败详情" description={job.error} />
            )}

            {job.state === 'completed' && (
              <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2">
                <span className="text-xs text-[var(--text-secondary)]">
                  容器已就绪，可继续下一步操作：
                </span>
                <div className="flex flex-wrap items-center gap-1.5">
                  {onOpenTerminal && (
                    <Button
                      size="sm"
                      variant="primary"
                      onClick={() => onOpenTerminal(job.container)}
                    >
                      <Terminal className="size-3.5" />
                      打开终端
                    </Button>
                  )}
                  {onOpenTools && (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => onOpenTools(job.container)}
                    >
                      <Wrench className="size-3.5" />
                      配置工具环境
                    </Button>
                  )}
                  {onBackToContainers && (
                    <Button size="sm" variant="ghost" onClick={onBackToContainers}>
                      返回容器列表
                      <ArrowRight className="size-3.5" />
                    </Button>
                  )}
                </div>
              </div>
            )}
          </Card>
        );
      })}
    </div>
  );
}
