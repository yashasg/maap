export type NotebookBlockType = 'heading' | 'paragraph' | 'checklist';

export interface NotebookBlock {
  id: string;
  type: NotebookBlockType;
  text: string;
  checked?: boolean;
}

export interface NotebookDocument {
  version: 1;
  blocks: NotebookBlock[];
}

export interface NotebookSummary {
  id: string;
  websiteId: string;
  createdBy: string | null;
  title: string;
  createdAt: string;
  updatedAt: string;
}

export interface Notebook extends NotebookSummary {
  content: NotebookDocument;
}

export function createBlock(type: NotebookBlockType = 'paragraph'): NotebookBlock {
  return {
    id: crypto.randomUUID(),
    type,
    text: '',
    ...(type === 'checklist' ? { checked: false } : {}),
  };
}

export function createDocument(): NotebookDocument {
  return { version: 1, blocks: [createBlock()] };
}

export function notebookExcerpt(document: NotebookDocument) {
  return document.blocks
    .map(block => block.text.trim())
    .filter(Boolean)
    .join(' · ');
}
