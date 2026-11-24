import { Button, Card, ProgressBar } from '@gojona/ui';
import { FolderUp } from 'lucide-react';
import { useState } from 'react';
import { formatBytes } from '../lib/format.ts';

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

  return (
    <Card
      className={`flex flex-col items-center justify-center gap-3 border-2 border-dashed p-8 text-center transition-colors ${
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
      <div>
        <div className="text-sm font-medium text-[var(--text-primary)]">
          {selectedFile
            ? selectedFile.name
            : '拖拽 .tar.gz / .tar.xz / .tar.zst / .img / .img.gz 到此处'}
        </div>
        <div className="mt-1 text-xs text-[var(--text-secondary)]">
          {selectedFile
            ? `文件大小 ${formatBytes(selectedFile.size)} · 流式直写，零临时文件占用`
            : '支持流式上传并自动构建 ext4 稀疏镜像与 Dropbear SSH'}
        </div>
      </div>
      <label className="cursor-pointer">
        <input
          type="file"
          className="hidden"
          accept=".gz,.xz,.zst,.tar,.img"
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
            value={Math.round((uploadProgress.loaded / Math.max(1, uploadProgress.total)) * 100)}
            showValue
            label={`上传中 ${formatBytes(uploadProgress.loaded)} / ${formatBytes(uploadProgress.total)}`}
          />
          <Button size="sm" variant="danger" className="mt-2" onClick={onCancelUpload}>
            取消上传
          </Button>
        </div>
      )}
    </Card>
  );
}
