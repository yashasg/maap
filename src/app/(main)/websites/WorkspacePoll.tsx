'use client';

import type { FormEvent } from 'react';
import { useEffect, useId, useState } from 'react';
import { useApi } from '@/components/hooks/useApi';
import { ChevronRight } from '@/components/icons';
import type { ApiError } from '@/lib/types';
import styles from './WorkspacePoll.module.css';

const DISCOVERY_CHOICES = [
  { value: 'search', label: 'Search or an app store' },
  { value: 'recommendation', label: 'A teammate or friend' },
  { value: 'community', label: 'A developer community' },
  { value: 'content', label: 'An article, video, or podcast' },
  { value: 'comparison', label: 'While comparing analytics tools' },
  { value: 'other', label: 'Something else' },
] as const;

type DiscoveryChoice = (typeof DISCOVERY_CHOICES)[number]['value'];
type DiscoveryResponse = { choice: DiscoveryChoice | null };

function getChoiceLabel(choice: DiscoveryChoice | null) {
  return DISCOVERY_CHOICES.find(option => option.value === choice)?.label;
}

export function WorkspacePoll() {
  const { get, post, useMutation, useQuery } = useApi();
  const groupId = useId();
  const [draft, setDraft] = useState<DiscoveryChoice | ''>('');
  const [savedChoice, setSavedChoice] = useState<DiscoveryChoice | null>(null);
  const [saveNotice, setSaveNotice] = useState('');

  const discoveryQuery = useQuery<DiscoveryResponse, ApiError>({
    queryKey: ['me:discovery'],
    queryFn: () => get('/me/discovery'),
  });
  const saveMutation = useMutation<DiscoveryResponse, ApiError, DiscoveryChoice>({
    mutationFn: choice => post('/me/discovery', { choice }),
  });

  useEffect(() => {
    if (discoveryQuery.data) {
      setSavedChoice(discoveryQuery.data.choice);
      setDraft(discoveryQuery.data.choice ?? '');
    }
  }, [discoveryQuery.data]);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSaveNotice('');
    saveMutation.reset();

    if (!draft) {
      return;
    }

    try {
      const result = await saveMutation.mutateAsync(draft);
      setSavedChoice(result.choice);
      setDraft(result.choice ?? '');
      setSaveNotice('Saved on this Maap instance.');
    } catch {
      // The mutation exposes the actionable error beside the form.
    }
  };

  const savedLabel = getChoiceLabel(savedChoice);
  const summaryText = discoveryQuery.isLoading
    ? 'Checking for a saved answer…'
    : discoveryQuery.error
      ? 'Your saved answer could not be loaded.'
      : savedLabel
        ? `Saved answer: ${savedLabel}`
        : 'Optional · about 20 seconds';

  return (
    <aside className={styles.card} aria-labelledby={`${groupId}-question`}>
      <details className={styles.disclosure}>
        <summary className={styles.summary}>
          <span className={styles.summaryCopy}>
            <span className={styles.badge}>Quick check-in</span>
            <strong id={`${groupId}-question`} className={styles.question}>
              What brought you to Maap?
            </strong>
            <span className={styles.summaryText}>{summaryText}</span>
          </span>
          <ChevronRight className={styles.arrow} aria-hidden="true" size={20} />
        </summary>

        <div className={styles.content}>
          {discoveryQuery.isLoading ? (
            <div className={styles.loading} role="status">
              <span className={styles.loadingDot} aria-hidden="true" />
              Loading your saved answer…
            </div>
          ) : discoveryQuery.error ? (
            <div className={styles.feedback} role="alert">
              <p>We couldn’t load your saved answer. Your existing choice was not changed.</p>
              <button
                className={styles.secondaryButton}
                type="button"
                onClick={() => discoveryQuery.refetch()}
              >
                Try again
              </button>
            </div>
          ) : (
            <form className={styles.form} onSubmit={handleSubmit}>
              <fieldset className={styles.fieldset}>
                <legend>Choose the closest match</legend>
                <div className={styles.options}>
                  {DISCOVERY_CHOICES.map(option => {
                    const optionId = `${groupId}-${option.value}`;

                    return (
                      <label className={styles.option} htmlFor={optionId} key={option.value}>
                        <input
                          id={optionId}
                          type="radio"
                          name={`${groupId}-discovery`}
                          value={option.value}
                          checked={draft === option.value}
                          onChange={() => {
                            setDraft(option.value);
                            setSaveNotice('');
                            saveMutation.reset();
                          }}
                        />
                        <span>{option.label}</span>
                      </label>
                    );
                  })}
                </div>
              </fieldset>

              <p className={styles.privacy}>
                This optional answer is stored only on this self-hosted Maap instance. You can
                change it anytime.
              </p>

              {saveMutation.error && (
                <p className={styles.error} role="alert">
                  We couldn’t save that choice. Check your connection, then try saving again.
                </p>
              )}
              {saveNotice && (
                <p className={styles.success} role="status" aria-live="polite">
                  {saveNotice}
                </p>
              )}

              <button
                className={styles.saveButton}
                type="submit"
                disabled={
                  !draft ||
                  draft === savedChoice ||
                  saveMutation.isPending ||
                  discoveryQuery.isFetching
                }
              >
                {saveMutation.isPending
                  ? 'Saving…'
                  : saveMutation.error
                    ? 'Try saving again'
                    : savedChoice
                      ? 'Update answer'
                      : 'Save answer'}
              </button>
            </form>
          )}
        </div>
      </details>
    </aside>
  );
}
