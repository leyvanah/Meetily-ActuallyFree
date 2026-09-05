"use client";

import { useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { Sparkles } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

export function SummaryRegenerationDialog({
  open,
  onOpenChange,
  initialContext = '',
  speakerNamesChanged = false,
  onRegenerate,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialContext?: string;
  speakerNamesChanged?: boolean;
  onRegenerate: (context: string) => Promise<void>;
}) {
  const t = useTranslations('meetingDetails');
  const suggestions = [
    t('suggestionActionItems'),
    t('suggestionShort'),
    t('suggestionDecisions'),
    t('suggestionSpeakerNames'),
  ];
  const [context, setContext] = useState('');
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (open) setContext(initialContext);
  }, [open, initialContext]);

  const submit = async () => {
    setSubmitting(true);
    try {
      await onRegenerate(context.trim());
      onOpenChange(false);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => !submitting && onOpenChange(nextOpen)}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles size={18} className="text-blue-500" />
            {speakerNamesChanged ? t('regenSpeakersTitle') : t('regenerateDialogTitle')}
          </DialogTitle>
          <DialogDescription>
            {speakerNamesChanged
              ? t('regenSpeakersDescription')
              : t('regenDescription')}
          </DialogDescription>
        </DialogHeader>

        <textarea
          autoFocus
          value={context}
          onChange={(event) => setContext(event.target.value)}
          onKeyDown={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
              event.preventDefault();
              void submit();
            }
          }}
          placeholder={t('regenPlaceholder')}
          rows={4}
          className="w-full resize-none rounded-md border border-[var(--af-border)] bg-[var(--af-panel-2)] px-3 py-2 text-sm text-[var(--af-text)] outline-none focus:ring-2 focus:ring-blue-500"
        />

        <div className="flex flex-wrap gap-1.5">
          {suggestions.map((suggestion) => (
            <button
              key={suggestion}
              type="button"
              onClick={() => setContext((current) => current.trim() ? `${current.trim()}\n${suggestion}` : suggestion)}
              className="rounded-full border border-[var(--af-border)] px-2.5 py-1 text-xs text-[var(--af-text-2)] transition-colors hover:border-blue-400 hover:text-blue-400"
            >
              + {suggestion}
            </button>
          ))}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" disabled={submitting} onClick={() => onOpenChange(false)}>
            {t('regenNotNow')}
          </Button>
          <Button type="button" disabled={submitting} onClick={() => void submit()}>
            {submitting ? t('regenStarting') : context.trim() ? t('regenWithContext') : t('regenerate')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
