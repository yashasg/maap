'use client';

import {
  Box,
  Button,
  Column,
  Form,
  FormField,
  FormSubmitButton,
  Heading,
  Loading,
  Row,
  Text,
  TextField,
} from '@umami/react-zen';
import { useMemo, useState } from 'react';
import { v4 as uuid } from 'uuid';
import { BoardViewBody } from '@/app/(main)/boards/[boardId]/BoardViewBody';
import { BoardProvider } from '@/app/(main)/boards/BoardProvider';
import { getComponentDefinition } from '@/app/(main)/boards/boardComponentRegistry';
import { EmptyPlaceholder } from '@/components/common/EmptyPlaceholder';
import { LinkButton } from '@/components/common/LinkButton';
import { Panel } from '@/components/common/Panel';
import {
  useApi,
  useLoginQuery,
  useMessages,
  useModified,
  useNavigation,
  useTeamMembersQuery,
  useWebsiteQuery,
} from '@/components/hooks';
import { ChevronLeft, ChevronRight, FileUp, Gauge, Plus } from '@/components/icons';
import { DialogButton } from '@/components/input/DialogButton';
import { BOARD_TYPES } from '@/lib/boards';
import { ROLES } from '@/lib/constants';
import type { Board, BoardColumn, BoardParameters, BoardRow, PageResult } from '@/lib/types';
import styles from './Screen.module.css';

const IMPORT_FORMAT = 'maap-board';
const IMPORT_VERSION = 1;
const IMPORT_FILE_LIMIT = 256 * 1024;
const PAGE_SIZE = 100;

interface ImportedBoard {
  name: string;
  description: string;
  parameters: BoardParameters;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

function readShortString(value: unknown, label: string, maxLength: number, required = false) {
  if (value == null && !required) {
    return '';
  }

  if (typeof value !== 'string' || (required && !value.trim()) || value.length > maxLength) {
    throw new Error(
      `${label} must be ${required ? 'a non-empty ' : 'a '}string under ${maxLength + 1} characters.`,
    );
  }

  return value.trim();
}

function sanitizeProps(
  value: unknown,
  definition: NonNullable<ReturnType<typeof getComponentDefinition>>,
) {
  if (value == null) {
    return undefined;
  }

  if (!isRecord(value)) {
    throw new Error(`Invalid configuration for ${definition.name}.`);
  }

  const fields = new Map((definition.configFields ?? []).map(field => [field.name, field]));
  const props: Record<string, string | number> = {};

  for (const [key, rawValue] of Object.entries(value)) {
    const field = fields.get(key);

    if (!field) {
      continue;
    }

    if (field.type === 'number') {
      if (typeof rawValue !== 'number' || !Number.isFinite(rawValue)) {
        throw new Error(`${definition.name} has an invalid ${field.label.toLowerCase()}.`);
      }
      props[key] = rawValue;
    } else {
      const stringValue = readShortString(rawValue, field.label, 2000, !!field.required);
      const options = field.optionsByEntityType?.website ?? field.options;

      if (
        field.type === 'select' &&
        options &&
        !options.some(option => option.value === stringValue)
      ) {
        throw new Error(`${definition.name} has an invalid ${field.label.toLowerCase()}.`);
      }

      if (
        field.type === 'report' &&
        stringValue &&
        !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
          stringValue,
        )
      ) {
        throw new Error(`${definition.name} has an invalid saved report reference.`);
      }

      props[key] = stringValue;
    }
  }

  return Object.keys(props).length ? props : undefined;
}

function sanitizeColumn(value: unknown): BoardColumn {
  if (!isRecord(value)) {
    throw new Error('Every dashboard column must be an object.');
  }

  if (value.component == null) {
    return { id: uuid(), component: undefined };
  }

  if (!isRecord(value.component)) {
    throw new Error('Every dashboard component must be an object.');
  }

  const type = readShortString(value.component.type, 'Component type', 100, true);
  const definition = getComponentDefinition(type);

  if (!definition) {
    throw new Error(`Unsupported app dashboard component: ${type}.`);
  }

  const size =
    typeof value.size === 'number' && Number.isFinite(value.size)
      ? Math.min(100, Math.max(1, value.size))
      : undefined;

  return {
    id: uuid(),
    ...(size ? { size } : {}),
    component: {
      type,
      title: readShortString(value.component.title, 'Component title', 100) || undefined,
      description:
        readShortString(value.component.description, 'Component description', 500) || undefined,
      props: sanitizeProps(value.component.props, definition),
    },
  };
}

function sanitizeRow(value: unknown): BoardRow {
  if (!isRecord(value) || !Array.isArray(value.columns) || value.columns.length > 4) {
    throw new Error('Each dashboard row must contain between one and four columns.');
  }

  if (value.columns.length === 0) {
    throw new Error('Dashboard rows cannot be empty.');
  }

  const size =
    typeof value.size === 'number' && Number.isFinite(value.size)
      ? Math.min(1200, Math.max(80, value.size))
      : undefined;

  return {
    id: uuid(),
    ...(size ? { size } : {}),
    columns: value.columns.map(sanitizeColumn),
  };
}

export function parseMaapBoardImport(value: unknown): ImportedBoard {
  if (!isRecord(value) || value.format !== IMPORT_FORMAT || value.version !== IMPORT_VERSION) {
    throw new Error(
      'Choose a version 1 Maap Board JSON file. Other dashboard formats are not supported.',
    );
  }

  if (!Array.isArray(value.rows) || value.rows.length > 20) {
    throw new Error('A Maap Board import can contain at most 20 rows.');
  }

  return {
    name: readShortString(value.name, 'Dashboard name', 100, true),
    description: readShortString(value.description, 'Dashboard description', 500),
    parameters: {
      rows: value.rows.map(sanitizeRow),
    },
  };
}

function NewDashboardForm({
  isPending,
  error,
  onCancel,
  onSubmit,
}: {
  isPending: boolean;
  error?: string;
  onCancel: () => void;
  onSubmit: (data: { name: string }) => void;
}) {
  return (
    <Form onSubmit={onSubmit} error={error} defaultValues={{ name: '' }}>
      <FormField
        name="name"
        label="Name"
        rules={{
          required: 'Required',
          validate: value => !!value.trim() || 'Enter a dashboard name.',
        }}
      >
        <TextField autoFocus autoComplete="off" maxLength={100} />
      </FormField>
      <Row justifyContent="flex-end" paddingTop="3">
        <Button type="button" variant="quiet" isDisabled={isPending} onPress={onCancel}>
          Cancel
        </Button>
        <FormSubmitButton variant="primary" isDisabled={isPending}>
          Create
        </FormSubmitButton>
      </Row>
    </Form>
  );
}

function ImportDashboardForm({
  onImport,
  onClose,
}: {
  onImport: (board: ImportedBoard) => Promise<Board>;
  onClose: () => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(false);
  const submit = async () => {
    if (!file || pending) return;
    setPending(true);
    setError('');
    try {
      if (file.size > IMPORT_FILE_LIMIT)
        throw new Error('Maap Board files must be 256 KB or smaller.');
      await onImport(parseMaapBoardImport(JSON.parse(await file.text())));
    } catch (e) {
      setError(
        e instanceof SyntaxError
          ? 'The file is not valid JSON.'
          : e instanceof Error
            ? e.message
            : 'Dashboard import failed.',
      );
      setPending(false);
      return;
    }
    onClose();
  };
  return (
    <form
      onSubmit={event => {
        event.preventDefault();
        void submit();
      }}
    >
      <Column gap="3">
        <label>
          Dashboard export file (.json)
          <input
            type="file"
            accept=".json,application/json"
            disabled={pending}
            onChange={event => {
              setFile(event.target.files?.[0] ?? null);
              setError('');
            }}
          />
        </label>
        <Text size="sm" color="muted">
          Use a version 1 Maap Board JSON export. TelemetryDeck exports are not supported.
        </Text>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        <Row justifyContent="flex-end" gap="2">
          <Button type="button" variant="quiet" isDisabled={pending} onPress={onClose}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" isDisabled={!file || pending}>
            Import
          </Button>
        </Row>
      </Column>
    </form>
  );
}

export default function DashboardsScreen({ websiteId }: { websiteId: string }) {
  const [isSidebarVisible, setIsSidebarVisible] = useState(true);
  const {
    data: website,
    isLoading: isWebsiteLoading,
    error: websiteError,
  } = useWebsiteQuery(websiteId);
  const { user } = useLoginQuery();
  const { get, post, useMutation, useQuery } = useApi();
  const { modified, touch } = useModified('boards');
  const { renderUrl, query, updateParams, router } = useNavigation();
  const selectedBoardId = query.dashboardId;
  const selectDashboard = (dashboardId: string) =>
    router.push(updateParams({ dashboardId }), { scroll: false });
  const { getErrorMessage } = useMessages();
  const teamId = website?.teamId ?? undefined;
  const { data: teamMembers } = useTeamMembersQuery(teamId ?? '');
  const canCreate =
    !!website &&
    !!user &&
    user.role !== ROLES.viewOnly &&
    (user.isAdmin ||
      !teamId ||
      teamMembers?.data.some(
        member => member.userId === user.id && member.role !== ROLES.teamViewOnly,
      ));

  const boardsQuery = useQuery<PageResult<Board[]>>({
    queryKey: ['app-dashboards', { websiteId, teamId, modified }],
    enabled: !!website && !websiteError,
    queryFn: async () => {
      const path = teamId ? `/teams/${teamId}/boards` : '/boards';
      const first = (await get(path, {
        page: 1,
        pageSize: PAGE_SIZE,
        orderBy: 'createdAt',
        sortDescending: true,
      })) as PageResult<Board[]>;
      const pageCount = Math.ceil(first.count / PAGE_SIZE);
      const rest =
        pageCount > 1
          ? await Promise.all(
              Array.from({ length: pageCount - 1 }, (_, index) =>
                get(path, {
                  page: index + 2,
                  pageSize: PAGE_SIZE,
                  orderBy: 'createdAt',
                  sortDescending: true,
                }),
              ),
            )
          : [];
      const data = [first, ...rest]
        .flatMap(page => page.data as Board[])
        .filter(
          board => board.type === BOARD_TYPES.website && board.parameters?.websiteId === websiteId,
        );

      return { ...first, data, count: data.length, page: 1, pageSize: data.length || PAGE_SIZE };
    },
  });

  const boards = boardsQuery.data?.data ?? [];
  const selectedBoard = useMemo(
    () => boards.find(board => board.id === selectedBoardId),
    [boards, selectedBoardId],
  );
  const createMutation = useMutation({
    mutationFn: (board: ImportedBoard) =>
      post('/boards', {
        name: board.name,
        description: board.description,
        type: BOARD_TYPES.website,
        teamId,
        parameters: {
          ...board.parameters,
          websiteId,
        },
      }),
    onSuccess: (board: Board) => {
      touch('boards');
      selectDashboard(board.id);
    },
  });

  const loading = isWebsiteLoading || boardsQuery.isLoading;
  const error = websiteError || boardsQuery.error;

  return (
    <Column gap="4">
      <Row justifyContent="space-between" alignItems="center" wrap="wrap" gap="3">
        <Column gap="1">
          <Heading>Dashboards</Heading>
          <Text color="muted">Build app-scoped views from Maap Board components.</Text>
        </Column>
        {!isSidebarVisible && (
          <Button variant="outline" onPress={() => setIsSidebarVisible(true)}>
            <Row gap="2" alignItems="center">
              <ChevronRight size={16} />
              <Text>Show sidebar</Text>
            </Row>
          </Button>
        )}
      </Row>

      <Box
        className={`${styles.workspace} ${!isSidebarVisible ? styles.workspaceExpanded : ''}`}
        style={{
          minHeight: 440,
        }}
      >
        {isSidebarVisible && (
          <Column gap="3">
            {canCreate && (
              <>
                <DialogButton
                  icon={<Plus />}
                  label="New Dashboard"
                  width="520px"
                  onPress={() => createMutation.reset()}
                >
                  {({ close }) => (
                    <NewDashboardForm
                      isPending={createMutation.isPending}
                      error={
                        createMutation.error ? getErrorMessage(createMutation.error) : undefined
                      }
                      onCancel={close}
                      onSubmit={({ name }) =>
                        createMutation.mutate(
                          {
                            name: name.trim(),
                            description: '',
                            parameters: {
                              rows: [{ id: uuid(), columns: [{ id: uuid(), component: null }] }],
                            },
                          },
                          { onSuccess: close },
                        )
                      }
                    />
                  )}
                </DialogButton>
                <DialogButton
                  icon={<FileUp />}
                  label="Import Dashboard"
                  variant="outline"
                  width="520px"
                  onPress={() => createMutation.reset()}
                >
                  {({ close }) => (
                    <ImportDashboardForm
                      onClose={close}
                      onImport={board => createMutation.mutateAsync(board)}
                    />
                  )}
                </DialogButton>
              </>
            )}
            <Button variant="quiet" onPress={() => setIsSidebarVisible(false)}>
              <Row gap="2" alignItems="center">
                <ChevronLeft size={16} />
                <Text>Hide sidebar</Text>
              </Row>
            </Button>
            {boards.length > 0 && (
              <Column gap="1" paddingTop="3">
                <Text color="muted" size="sm" weight="bold">
                  SAVED DASHBOARDS
                </Text>
                {boards.map(board => (
                  <Button
                    key={board.id}
                    variant={selectedBoardId === board.id ? 'primary' : 'quiet'}
                    onPress={() => selectDashboard(board.id)}
                  >
                    {board.name}
                  </Button>
                ))}
              </Column>
            )}
          </Column>
        )}

        <Panel minHeight="440px">
          {loading ? (
            <Loading />
          ) : error ? (
            <EmptyPlaceholder
              title="Dashboards could not be loaded"
              description={getErrorMessage(error)}
              minHeight="360px"
            />
          ) : selectedBoard ? (
            <Column gap="4">
              <Row justifyContent="space-between" alignItems="center" wrap="wrap" gap="3">
                <Column gap="1">
                  <Heading>{selectedBoard.name}</Heading>
                  {selectedBoard.description && (
                    <Text color="muted">{selectedBoard.description}</Text>
                  )}
                </Column>
                {canCreate && (
                  <LinkButton
                    href={renderUrl(`/boards/${selectedBoard.id}/design`, false)}
                    variant="outline"
                  >
                    Design dashboard
                  </LinkButton>
                )}
              </Row>
              <BoardProvider boardId={selectedBoard.id}>
                <BoardViewBody showEntityBadges={false} />
              </BoardProvider>
            </Column>
          ) : (
            <EmptyPlaceholder
              icon={<Gauge />}
              title={boards.length ? 'Open a dashboard' : 'Create your first dashboard'}
              description={
                boards.length
                  ? 'Choose a saved dashboard from the sidebar.'
                  : canCreate
                    ? 'Dashboards let you arrange supported Maap Board components for this app.'
                    : 'No dashboards are available for this app. Ask an editor to create one.'
              }
              minHeight="360px"
            />
          )}
        </Panel>
      </Box>
    </Column>
  );
}
