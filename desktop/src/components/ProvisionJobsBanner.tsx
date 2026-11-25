import { Badge, Button, Card, ProgressBar } from '@gojona/ui';
import { XCircle } from 'lucide-react';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type { ContainerJobProgressPayload } from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';

export interface ProvisionJobsBannerProps {
  transport: StrataTransport | null;
  jobs: ContainerJobProgressPayload[];
}

/**
 * Displays live progress and cancellation controls for asynchronous container provisioning jobs.
 */
export function ProvisionJobsBanner({ transport, jobs }: ProvisionJobsBannerProps) {
  if (jobs.length === 0) return null;

  return (
    <div className="flex flex-col gap-2">
      {jobs.map((job) => (
        <Card key={job.jobId} className="flex flex-col gap-2 p-3.5">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium text-[var(--text-primary)]">
                {job.container}
              </span>
              <Badge
                variant={
                  job.state === 'completed'
                    ? 'success'
                    : job.state === 'failed'
                      ? 'danger'
                      : 'accent'
                }
              >
                {job.stage}
              </Badge>
            </div>
            {job.state === 'running' && transport && (
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
          </div>
          <ProgressBar
            value={job.percent ?? 0}
            showValue
            label={job.message ?? '处理中…'}
            tone={job.state === 'failed' ? 'danger' : 'accent'}
          />
        </Card>
      ))}
    </div>
  );
}
