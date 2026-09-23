"use client";

/**
 * Questions about one meeting, answered from its transcript by the local
 * model. Its own tab beside the summary, so the question box does not sit
 * under the summary all the time.
 *
 * The conversation lives as long as the page does: the tab is hidden, not
 * unmounted, when another one is chosen.
 */

import { useMemo, useRef, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { MessageSquare, Send } from 'lucide-react';
import { Transcript } from '@/types';

const MAX_CONTEXT_CHARS = 6000;

interface QA {
  id: number;
  question: string;
  answer: string;
  status: 'pending' | 'done' | 'error';
}

export function MeetingChat({ transcripts }: { transcripts: Transcript[] }) {
  const t = useTranslations('meetingDetails');
  const [question, setQuestion] = useState('');
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<QA[]>([]);
  const nextId = useRef(1);

  const transcriptContext = useMemo(() => {
    const joined = transcripts
      .map((line) => (line.speaker ? `${line.speaker}: ` : '') + (line.text ?? ''))
      .filter(Boolean)
      .join('\n');
    return joined.length > MAX_CONTEXT_CHARS ? joined.slice(-MAX_CONTEXT_CHARS) : joined;
  }, [transcripts]);

  const ask = async () => {
    const q = question.trim();
    if (!q || busy) return;
    const id = nextId.current++;
    setQuestion('');
    setHistory((prev) => [...prev, { id, question: q, answer: '', status: 'pending' }]);
    setBusy(true);
    try {
      const answer = await invoke<string>('ask_live_assistant', { question: q, transcriptContext, persona: null });
      setHistory((prev) => prev.map((x) => (x.id === id ? { ...x, answer, status: 'done' } : x)));
    } catch (err) {
      const msg = typeof err === 'string' ? err : (err as any)?.message || t('askRequestFailed');
      setHistory((prev) => prev.map((x) => (x.id === id ? { ...x, answer: `⚠️ ${msg}`, status: 'error' } : x)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-6 py-5">
        {history.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-sm text-[var(--af-text-3)]">
            <MessageSquare size={20} />
            <p className="max-w-xs">{t('chatEmpty')}</p>
          </div>
        ) : (
          history.map((qa) => (
            <div key={qa.id} className="space-y-1">
              <div className="ml-auto flex w-fit max-w-[85%] items-center gap-1 rounded-lg bg-[var(--af-panel-2)] px-3 py-1.5 text-sm text-[var(--af-text)]">
                {qa.question}
              </div>
              <div className="w-fit max-w-[92%] px-1 py-1 text-sm text-[var(--af-text)]">
                {qa.status === 'pending' ? (
                  <span className="inline-flex items-center gap-1 text-[var(--af-text-3)]">
                    <span className="h-2 w-2 animate-pulse rounded-full bg-[var(--af-text-3)]" /> {t('thinking')}
                  </span>
                ) : (
                  <div className="prose prose-sm max-w-none dark:prose-invert prose-p:my-1 prose-ul:my-1">
                    <ReactMarkdown remarkPlugins={[remarkGfm]}>{qa.answer}</ReactMarkdown>
                  </div>
                )}
              </div>
            </div>
          ))
        )}
      </div>

      <div className="px-4 pb-4">
        <div className="flex items-center gap-2 rounded-xl border border-[var(--af-border)] bg-[var(--af-panel)] py-1.5 pl-4 pr-1.5 focus-within:border-[var(--af-border-strong)]">
          <textarea
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                void ask();
              }
            }}
            rows={1}
            placeholder={t('askAiPlaceholder')}
            className="af-bare flex-1 resize-none border-0 bg-transparent py-1 text-sm text-[var(--af-text)] placeholder:text-[var(--af-text-3)] focus:outline-none focus:ring-0"
          />
          <button
            type="button"
            onClick={() => void ask()}
            disabled={busy || !question.trim()}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] disabled:opacity-40 disabled:hover:bg-transparent"
            title={t('askAi')}
            aria-label={t('askAi')}
          >
            <Send size={16} />
          </button>
        </div>
      </div>
    </div>
  );
}
