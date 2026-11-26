import { Badge, Button, Card, ProgressBar } from '@gojona/ui';
import { FolderUp } from 'lucide-react';
import { useState } from 'react';
import { formatBytes } from '../lib/format.ts';
import { isRawDiskImageFilename } from './install/installHelpers.ts';

export interface LocalImportDropzoneProps {
  selectedFile: File | null;
  uploadProgress: { loaded: number; total: number } | null;
  onSelectFile: (file: File) => void;
  onCancelUpload: () => void;
}

/**
 * Drag-and-drop file selector and streaming upload progress panel for `.tar.*` / `.img` imports.
 */
export function LocalImportDropzone({
  selectedFile,
  uploadProgress,
  onSelectFile,
  onCancelUpload,
}: LocalImportDropzoneProps) {
  const [dragActive, setDragActive] = useState(false);

  const isRaw = selectedFile ? isRawDiskImageFilename(selectedFile.name) : false;
  const uploadPct = uploadProgress
    ? Math.round((uploadProgress.loaded / Math.max(1, uploadProgress.total)) * 100)
    : 0;
  const awaitingServer = uploadProgress !== null && uploadPct >= 100;

  return (
    <Card
      className={`flex flex-col items-center justify-center gap-3 border-2 border-dashed p-7 text-center transition-colors ${
        dragActive
          ? 'border-[var(--accent)] bg-[var(--accent)]/10'
          : 'border-[var(--border-hairline)]'
      }`}
      onDragOver={(e) => {
        e.preventDefault();
        setDragActive(true);
      }}
      onDragLeave={() => setDragActive(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragActive(false);
        const dropped = e.dataTransfer.files[0];
        if (dropped) onSelectFile(dropped);
      }}
    >
      <FolderUp className="size-9 text-[var(--accent)]" />
      <div className="flex flex-col items-center gap-1">
        <div className="flex flex-wrap items-center justify-center gap-2 text-sm font-medium text-[var(--text-primary)]">
          <span>
            {selectedFile
              ? selectedFile.name
              : '拖拽 .tar.gz / .tar.xz / .tar.zst / .tar / .img / .img.gz 到此处'}
          </span>
          {selectedFile && (
            <Badge variant={isRaw ? 'warning' : 'accent'}>
              {isRaw ? '原始磁盘镜像' : 'RootFS 归档包'}
            </Badge>
          )}
        </div>
        <div className="text-xs text-[var(--text-secondary)]">
          {selectedFile
            ? `文件大小 ${formatBytes(selectedFile.size)} · 流式直写，零临时文件占用`
            : '归档包将自动创建 ext4 稀疏镜像；.img/.img.gz 将校验超级块并直接导入'}
        </div>
      </div>
      <label className="cursor-pointer">
        <input
          type="file"
          className="hidden"
          accept=".gz,.xz,.zst,.tar,.img,.ext4"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) onSelectFile(file);
          }}
        />
        <span className="inline-flex items-center rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-1.5 text-xs font-medium text-[var(--text-primary)] hover:border-[var(--accent)]">
          选择本地镜像文件
        </span>
      </label>
      {uploadProgress && (
        <div className="w-full max-w-md">
          <ProgressBar
            value={uploadPct}
            showValue
            label={
              awaitingServer
                ? '文件已发送完毕，服务端正在解压与初始化容器（请留意上方进度）…'
                : `上传中 ${formatBytes(uploadProgress.loaded)} / ${formatBytes(uploadProgress.total)}`
            }
          />
          {!awaitingServer && (
            <Button size="sm" variant="danger" className="mt-2" onClick={onCancelUpload}>
              取消上传
            </Button>
          )}
        </div>
      )}
    </Card>
  );
}
