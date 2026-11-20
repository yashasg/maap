'use client';

import { useQueryClient } from '@tanstack/react-query';
import { Button, Column, Heading, Row, Text } from '@umami/react-zen';
import { formatDistanceToNow } from 'date-fns';
import { ArrowLeft, CheckSquare, FileText, Heading2, Plus, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { LoadingPanel } from '@/components/common/LoadingPanel';
import { SectionHeader } from '@/components/common/SectionHeader';
import { useApi } from '@/components/hooks/useApi';
import { useNavigation } from '@/components/hooks/useNavigation';
import {
  createBlock,
  createDocument,
  type Notebook,
  type NotebookBlock,
  type NotebookBlockType,
  type NotebookDocument,
  type NotebookSummary,
} from './document';
import styles from './Notebooks.module.css';

interface NotebookListResponse {
  data: NotebookSummary[];
  canWrite: boolean;
}

interface NotebookResponse {
  data: Notebook;
  canWrite: boolean;
}

const listKey = (websiteId: string) => ['app-notebooks', websiteId];
const detailKey = (websiteId: string, notebookId: string) => [
  'app-notebooks',
  websiteId,
  notebookId,
];

export default function NotebooksScreen({ websiteId }: { websiteId: string }) {
  const { get, post, useMutation, useQuery } = useApi();
  const queryClient = useQueryClient();
  const { query, router, updateParams } = useNavigation();
  const selectedId = typeof query.notebook === 'string' ? query.notebook : undefined;

  const listQuery = useQuery<NotebookListResponse>({
    queryKey: listKey(websiteId),
    queryFn: () => get(`/websites/${websiteId}/notebooks`),
  });

  const detailQuery = useQuery<NotebookResponse>({
    queryKey: detailKey(websiteId, selectedId || ''),
    queryFn: () => get(`/websites/${websiteId}/notebooks/${selectedId}`),
    enabled: !!selectedId,
  });

  const createMutation = useMutation({
    mutationFn: () =>
      post(`/websites/${websiteId}/notebooks`, {
        title: 'Untitled notebook',
        content: createDocument(),
      }),
    onSuccess: async (result: Notebook) => {
      await queryClient.invalidateQueries({ queryKey: listKey(websiteId) });
      router.replace(updateParams({ notebook: result.id }), { scroll: false });
    },
  });

  if (selectedId) {
    return (
      <LoadingPanel
        className={styles.screen}
        minHeight="430px"
        data={detailQuery.data}
        isLoading={detailQuery.isLoading}
        error={detailQuery.error}
      >
        {detailQuery.data && (
          <NotebookEditor
            key={detailQuery.data.data.id}
            notebook={detailQuery.data.data}
            canWrite={detailQuery.data.canWrite}
            websiteId={websiteId}
            onBack={() => router.replace(updateParams({ notebook: undefined }), { scroll: false })}
          />
        )}
      </LoadingPanel>
    );
  }

  return (
    <Column className={`${styles.screen} ${styles.landing}`} gap>
      <SectionHeader title="Notebooks">
        <Button
          variant="primary"
          isDisabled={!listQuery.data?.canWrite || createMutation.isPending}
          onPress={() => createMutation.mutate()}
        >
          <Plus size={16} aria-hidden />
          Create New
        </Button>
      </SectionHeader>

      {createMutation.error && (
        <div className={styles.error} role="alert">
          The notebook could not be created. Check your access and try again.
        </div>
      )}

      <LoadingPanel
        minHeight="320px"
        data={listQuery.data}
        isLoading={listQuery.isLoading}
        error={listQuery.error}
        isEmpty={false}
      >
        {listQuery.data?.data.length ? (
          <div className={styles.grid}>
            {listQuery.data.data.map(notebook => (
              <button
                className={styles.notebookCard}
                key={notebook.id}
                type="button"
                onClick={() =>
                  router.replace(updateParams({ notebook: notebook.id }), { scroll: false })
                }
              >
                <span className={styles.cardTitle}>{notebook.title}</span>
                <Text color="muted">Open this notebook to continue writing.</Text>
                <span className={styles.cardMeta}>
                  Updated{' '}
                  {formatDistanceToNow(new Date(notebook.updatedAt || notebook.createdAt), {
                    addSuffix: true,
                  })}
                </span>
              </button>
            ))}
          </div>
        ) : (
          <Column className={styles.empty} alignItems="center" justifyContent="center" gap="3">
            <div className={styles.emptyIcon}>
              <FileText size={24} aria-hidden />
            </div>
            <Heading size="lg">Keep analysis notes with this app</Heading>
            <Text color="muted" align="center">
              Create a notebook for findings, questions, and follow-up work.
            </Text>
            <Button
              isDisabled={!listQuery.data?.canWrite || createMutation.isPending}
              onPress={() => createMutation.mutate()}
            >
              Create your first notebook
            </Button>
          </Column>
        )}
      </LoadingPanel>
    </Column>
  );
}

function NotebookEditor({
  notebook,
  canWrite,
  websiteId,
  onBack,
}: {
  notebook: Notebook;
  canWrite: boolean;
  websiteId: string;
  onBack: () => void;
}) {
  const { post, useMutation } = useApi();
  const queryClient = useQueryClient();
  const [title, setTitle] = useState(notebook.title);
  const [content, setContent] = useState<NotebookDocument>(notebook.content);
  const [saved, setSaved] = useState({ title: notebook.title, content: notebook.content });

  const dirty = useMemo(
    () => title !== saved.title || JSON.stringify(content) !== JSON.stringify(saved.content),
    [content, saved, title],
  );

  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (!dirty) return;
      event.preventDefault();
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty]);

  const saveMutation = useMutation({
    mutationFn: () =>
      post(`/websites/${websiteId}/notebooks/${notebook.id}`, {
        title: title.trim(),
        content,
      }),
    onSuccess: async (result: Notebook) => {
      setTitle(result.title);
      setContent(result.content);
      setSaved({ title: result.title, content: result.content });
      queryClient.setQueryData(detailKey(websiteId, notebook.id), {
        data: result,
        canWrite,
      });
      await queryClient.invalidateQueries({ queryKey: listKey(websiteId) });
    },
  });

  const updateBlock = (id: string, update: Partial<NotebookBlock>) => {
    setContent(current => ({
      ...current,
      blocks: current.blocks.map(block => (block.id === id ? { ...block, ...update } : block)),
    }));
  };

  const removeBlock = (id: string) => {
    setContent(current => {
      const blocks = current.blocks.filter(block => block.id !== id);
      return { ...current, blocks: blocks.length ? blocks : [createBlock()] };
    });
  };

  const addBlock = (type: NotebookBlockType) => {
    setContent(current => ({ ...current, blocks: [...current.blocks, createBlock(type)] }));
  };

  const leaveEditor = () => {
    if (!dirty || window.confirm('Discard unsaved changes?')) onBack();
  };

  return (
    <Column className={styles.editor} gap>
      <SectionHeader>
        <Button variant="quiet" onPress={leaveEditor}>
          <ArrowLeft size={16} aria-hidden />
          All notebooks
        </Button>
        <Row gap="3" alignItems="center">
          <Text color="muted">
            {!canWrite
              ? 'Read only'
              : saveMutation.isPending
                ? 'Saving…'
                : dirty
                  ? 'Unsaved changes'
                  : 'Saved'}
          </Text>
          <Button
            variant="primary"
            isDisabled={!canWrite || !dirty || !title.trim() || saveMutation.isPending}
            onPress={() => saveMutation.mutate()}
          >
            Save notebook
          </Button>
        </Row>
      </SectionHeader>

      {saveMutation.error && (
        <div className={styles.error} role="alert">
          Changes were not saved. Your draft is still here; try again.
        </div>
      )}

      <article className={styles.editorPanel} aria-label="Notebook editor">
        <input
          className={styles.titleInput}
          aria-label="Notebook title"
          maxLength={200}
          placeholder="Untitled notebook"
          value={title}
          disabled={!canWrite}
          onChange={event => setTitle(event.target.value)}
        />

        {content.blocks.map((block, index) => (
          <div className={styles.block} key={block.id}>
            {block.type === 'checklist' ? (
              <div className={styles.checklist}>
                <label className={styles.checkControl}>
                  <input
                    className={styles.checkbox}
                    type="checkbox"
                    aria-label={`Complete item ${index + 1}`}
                    checked={!!block.checked}
                    disabled={!canWrite}
                    onChange={event => updateBlock(block.id, { checked: event.target.checked })}
                  />
                </label>
                <textarea
                  className={`${styles.blockInput} ${block.checked ? styles.checked : ''}`}
                  aria-label={`Checklist item ${index + 1}`}
                  rows={2}
                  maxLength={10000}
                  placeholder="Checklist item"
                  value={block.text}
                  disabled={!canWrite}
                  onChange={event => updateBlock(block.id, { text: event.target.value })}
                />
              </div>
            ) : (
              <textarea
                className={`${styles.blockInput} ${
                  block.type === 'heading' ? styles.headingInput : ''
                }`}
                aria-label={`${block.type === 'heading' ? 'Heading' : 'Text'} block ${index + 1}`}
                rows={block.type === 'heading' ? 1 : 3}
                maxLength={10000}
                placeholder={block.type === 'heading' ? 'Heading' : 'Write a note…'}
                value={block.text}
                disabled={!canWrite}
                onChange={event => updateBlock(block.id, { text: event.target.value })}
              />
            )}

            {canWrite && content.blocks.length > 1 && (
              <Row className={styles.blockActions}>
                <Button
                  variant="quiet"
                  aria-label={`Remove block ${index + 1}`}
                  onPress={() => removeBlock(block.id)}
                >
                  <Trash2 size={15} aria-hidden />
                </Button>
              </Row>
            )}
          </div>
        ))}

        {canWrite && (
          <Row className={styles.addBar} gap="2" wrap="wrap">
            <Button variant="quiet" onPress={() => addBlock('paragraph')}>
              <Plus size={15} aria-hidden />
              Text
            </Button>
            <Button variant="quiet" onPress={() => addBlock('heading')}>
              <Heading2 size={15} aria-hidden />
              Heading
            </Button>
            <Button variant="quiet" onPress={() => addBlock('checklist')}>
              <CheckSquare size={15} aria-hidden />
              Checklist
            </Button>
          </Row>
        )}
      </article>
    </Column>
  );
}
