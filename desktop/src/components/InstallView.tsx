import { Button } from '@gojona/ui';
import { ArrowLeft, Copy, Download, FolderUp, Link2 } from 'lucide-react';
import { useMemo } from 'react';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type { ContainerJobProgressPayload, ContainerStatusSnapshot } from '../types/protocol.ts';
import { InstallConfigForm } from './install/InstallConfigForm.tsx';
import { InstallSourceSelector } from './install/InstallSourceSelector.tsx';
import { suggestContainerId } from './install/installHelpers.ts';
import { useInstallWorkflow } from './install/useInstallWorkflow.ts';
import { ProvisionJobsBanner } from './ProvisionJobsBanner.tsx';

export interface InstallViewProps {
  transport: StrataTransport | null;
  containers?: ContainerStatusSnapshot[];
  activeJobs: ContainerJobProgressPayload[];
  onDismissJob?: (jobId: string) => void;
  onRefresh: () => Promise<void> | void;
  onBackToContainers?: () => void;
  onOpenTerminal?: (containerId: string) => void;
  onOpenTools?: (containerId: string) => void;
  runAction: <T>(
    label: string,
    action: () => Promise<T>,
    successMessage?: string,
  ) => Promise<T | null>;
}

/** Unified 4-source container provisioning screen with validation, error recovery & post-install actions. */
export function InstallView({
  transport,
  containers = [],
  activeJobs,
  onDismissJob,
  onRefresh,
  onBackToContainers,
  onOpenTerminal,
  onOpenTools,
  runAction,
}: InstallViewProps) {
  const wf = useInstallWorkflow(transport, containers, onRefresh, runAction);

  const combinedJobs = useMemo(() => {
    if (!wf.localDoneJob || activeJobs.some((j) => j.jobId === wf.localDoneJob?.jobId)) {
      return activeJobs;
    }
    return [wf.localDoneJob, ...activeJobs];
  }, [activeJobs, wf.localDoneJob]);

  const commonInvalid = Boolean(!transport || wf.idError || wf.sshPortError || wf.urlOrShaError);

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          {onBackToContainers && (
            <Button size="sm" variant="ghost" onClick={onBackToContainers}>
              <ArrowLeft className="size-3.5" />
              返回容器列表
            </Button>
          )}
          <div>
            <h1 className="text-lg font-semibold text-[var(--text-primary)]">安装与导入容器</h1>
            <p className="text-xs text-[var(--text-secondary)]">
              支持在线发行版商店、HTTPS 直链、本地压缩归档 / .img 镜像与现有容器克隆
            </p>
          </div>
        </div>
      </div>

      <ProvisionJobsBanner
        transport={transport}
        jobs={combinedJobs}
        onRetryJob={() => {
          if (wf.sourceMode === 'local' && wf.selectedFile) void wf.handleUploadLocal();
          else if (wf.sourceMode === 'clone') void wf.handleSubmitClone();
          else void wf.handleSubmitOnlineOrUrl();
        }}
        onDismissJob={(jobId) => {
          if (wf.localDoneJob?.jobId === jobId) wf.setLocalDoneJob(null);
          else onDismissJob?.(jobId);
        }}
        {...(onOpenTerminal ? { onOpenTerminal } : {})}
        {...(onOpenTools ? { onOpenTools } : {})}
        {...(onBackToContainers ? { onBackToContainers } : {})}
      />

      <InstallSourceSelector
        sourceMode={wf.sourceMode}
        onSourceModeChange={wf.setSourceMode}
        mirror={wf.mirror}
        onMirrorChange={wf.setMirror}
        customMirrorUrl={wf.customMirrorUrl}
        onCustomMirrorUrlChange={wf.setCustomMirrorUrl}
        catalog={wf.catalog}
        loadingCatalog={wf.loadingCatalog}
        selectedDistro={wf.selectedDistro}
        onSelectDistro={(item) => {
          wf.setSelectedDistro(item);
          const suggested = suggestContainerId(`${item.os}-${item.release}`, containers);
          wf.setForm((f) => ({
            ...f,
            containerId: suggested,
            ...(f.hostnameLinked ? { hostname: suggested } : {}),
          }));
        }}
        onRefreshCatalog={() => void wf.loadCatalog(true)}
        sourceUrl={wf.sourceUrl}
        onSourceUrlChange={wf.setSourceUrl}
        expectedSha256={wf.expectedSha256}
        onExpectedSha256Change={wf.setExpectedSha256}
        selectedFile={wf.selectedFile}
        uploadProgress={wf.uploadProgress}
        onSelectFile={(file) => {
          wf.setSelectedFile(file);
          const rawBase = file.name.replace(/\.(tar\.(gz|xz|zst)|tar|img(\.gz)?|ext4)$/i, '');
          const suggested = suggestContainerId(rawBase, containers);
          wf.setForm((f) => ({
            ...f,
            containerId: suggested,
            ...(f.hostnameLinked ? { hostname: suggested } : {}),
          }));
        }}
        onCancelUpload={() => wf.abortRef.current?.abort()}
        containers={containers}
        cloneSourceId={wf.cloneSourceId}
        onCloneSourceIdChange={wf.setCloneSourceId}
      />

      <InstallConfigForm
        sourceMode={wf.sourceMode}
        isRawImg={wf.isRawImg}
        values={wf.form}
        idError={wf.idError}
        sshPortError={wf.sshPortError}
        urlOrShaError={wf.urlOrShaError}
        onChange={(patch) => wf.setForm((prev) => ({ ...prev, ...patch }))}
        submitAction={
          wf.sourceMode === 'online' ? (
            <Button
              variant="primary"
              disabled={commonInvalid || !wf.selectedDistro}
              onClick={() => void wf.handleSubmitOnlineOrUrl()}
            >
              <Download className="size-4" />
              开始安装容器
            </Button>
          ) : wf.sourceMode === 'url' ? (
            <Button
              variant="primary"
              disabled={commonInvalid}
              onClick={() => void wf.handleSubmitOnlineOrUrl()}
            >
              <Link2 className="size-4" />
              从直链拉取并创建
            </Button>
          ) : wf.sourceMode === 'local' ? (
            <Button
              variant="primary"
              disabled={commonInvalid || !wf.selectedFile || wf.uploadProgress !== null}
              onClick={() => void wf.handleUploadLocal()}
            >
              <FolderUp className="size-4" />
              流式上传并创建
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={
                !transport || !wf.cloneSourceId || Boolean(wf.idError) || Boolean(wf.sshPortError)
              }
              onClick={() => void wf.handleSubmitClone()}
            >
              <Copy className="size-4" />
              开始克隆容器
            </Button>
          )
        }
      />
    </div>
  );
}
