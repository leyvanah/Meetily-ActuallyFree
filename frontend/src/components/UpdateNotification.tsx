import React from 'react';
import { Download } from 'lucide-react';
import { toast } from 'sonner';
import { getLocaleMessages } from '@/contexts/LocaleContext';
import { UpdateInfo } from '@/services/updateService';

let globalShowDialogCallback: (() => void) | null = null;

export function setUpdateDialogCallback(callback: () => void) {
  globalShowDialogCallback = callback;
}

export function showUpdateNotification(updateInfo: UpdateInfo, onUpdateClick?: () => void) {
  // Called imperatively from outside the React tree, so read the catalog directly.
  const m = getLocaleMessages().app;

  const handleClick = () => {
    toast.dismiss(toastId);
    if (onUpdateClick) {
      onUpdateClick();
    } else if (globalShowDialogCallback) {
      globalShowDialogCallback();
    }
  };

  const toastId = toast.info(
    <div className="flex items-center justify-between gap-4">
      <div className="flex items-center gap-2">
        <Download className="h-4 w-4" />
        <div>
          <p className="font-medium">{m.updateAvailableTitle}</p>
          <p className="text-sm text-muted-foreground">
            {m.updateAvailableVersion.replace('{version}', updateInfo.version ?? '')}
          </p>
        </div>
      </div>
      <button
        onClick={(e) => {
          e.stopPropagation();
          handleClick();
        }}
        className="text-sm font-medium text-blue-600 hover:text-blue-700 underline"
      >
        {m.updateViewDetails}
      </button>
    </div>,
    {
      duration: 10000,
      position: 'bottom-center',
    }
  );
}
