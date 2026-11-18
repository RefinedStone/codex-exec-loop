export type LinearStatus = { connected: false } | { connected: true; viewerName: string };

export interface LinearIssue {
  readonly id: string;
  readonly identifier: string;
  readonly title: string;
  readonly url: string;
  readonly priority: number;
  readonly updatedAt: string;
  readonly branchName: string | null;
  readonly state: { readonly id: string; readonly name: string; readonly type: string };
  readonly team: { readonly id: string; readonly key: string; readonly name: string };
  readonly project: { readonly id: string; readonly name: string } | null;
  readonly cycle: { readonly id: string; readonly number: number; readonly name: string | null; readonly isActive: boolean } | null;
}

export interface LinearFilters {
  readonly teamId: string | null;
  readonly projectId: string | null;
  readonly currentCycle: boolean;
}

export interface LinearSummary {
  readonly issues: readonly LinearIssue[];
  readonly teams: readonly { readonly id: string; readonly name: string }[];
  readonly projects: readonly { readonly id: string; readonly name: string }[];
  readonly totalCached: number;
  readonly limit: 30;
}

export interface LinearBridge {
  status(): Promise<LinearStatus>;
  connect(token: string): Promise<LinearStatus>;
  disconnect(): Promise<LinearStatus>;
  summary(filters: LinearFilters, refresh: boolean): Promise<LinearSummary>;
  issuePrompt(id: string): Promise<string>;
  openExternal(url: string): Promise<void>;
}
