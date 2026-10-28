import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import {
  Archive,
  ArrowUpDown,
  createElement,
  createIcons,
  Info,
  LayoutGrid,
  Minus,
  RotateCcw,
  ServerCog,
  Settings,
  Square,
  Trash,
  UsersRound,
  X,
} from "lucide";
import brandIcon from "../../icons/icon.png";
import { renderAgentStrip } from "./agent-controls";
import {
  mcpCommandLine,
  type AgentProblem,
  type IntegrationInfo,
  type IntegrationMutation,
  type IntegrationResource,
} from "./integration";
import {
  filteredWidgets,
  isEmpty,
  previewKey,
  previewNeedsRefresh,
  sortedActiveWidgets,
  sortedDeletedWidgets,
  type DeletedWidgetSummary,
  type MainState,
  type WidgetFilter,
  type WidgetSortOrder,
  type WidgetSummary,
} from "./main-model";
import { translate, type Locale, type MessageKey } from "./main-locale";
import { createMainStartup } from "./main-startup";
import { createPreviewRefresh } from "./preview-refresh";
import {
  saveLocale,
  saveTheme,
  resolvedLocale,
  storedLocale,
  storedTheme,
  type Theme,
} from "./main-preferences";
import { createSegmentedControl } from "./segmented-control";
import { renderMiddleEllipsis } from "./text-ellipsis";

interface WidgetView {
  root: HTMLElement;
  image: HTMLImageElement;
  placeholder: HTMLElement;
  title: HTMLElement;
  size: HTMLElement;
  toggle: HTMLButtonElement;
  archiveButton: HTMLButtonElement;
  widget: WidgetSummary;
}

interface DeletedWidgetView {
  root: HTMLElement;
  image: HTMLImageElement;
  placeholder: HTMLElement;
  title: HTMLElement;
  size: HTMLElement;
  restoreButton: HTMLButtonElement;
  deleteButton: HTMLButtonElement;
  trashName: string;
}

type Page =
  | "local-widgets"
  | "community-widgets"
  | "integrations"
  | "general"
  | "about";

const grid = document.querySelector<HTMLElement>("[data-grid]")!;
const deletedGrid = document.querySelector<HTMLElement>("[data-deleted-grid]")!;
const empty = document.querySelector<HTMLElement>("[data-empty]")!;
const emptyTitle = document.querySelector<HTMLElement>("[data-empty-title]")!;
const emptyDescription = document.querySelector<HTMLElement>("[data-empty-description]")!;
const fault = document.querySelector<HTMLElement>("[data-fault]")!;
const currentGroup = document.querySelector<HTMLElement>("[data-current-group]")!;
const currentPage = document.querySelector<HTMLElement>("[data-current-page]")!;
const filterOptions = document.querySelector<HTMLElement>("[data-filter-options]")!;
const filterControlRoot = document.querySelector<HTMLElement>("[data-widget-filter-control]")!;
const themeControlRoot = document.querySelector<HTMLElement>("[data-theme-options]")!;
const recycleFilter = document.querySelector<HTMLButtonElement>("[data-recycle-filter]")!;
const widgetSortButton = document.querySelector<HTMLButtonElement>("[data-widget-sort]")!;
const widgetSortLabel = widgetSortButton.querySelector<HTMLElement>("[data-sort-label]")!;
const brandMark = document.querySelector<HTMLImageElement>("[data-brand-icon]")!;
const version = document.querySelector<HTMLElement>("[data-version]")!;
const updateStatus = document.querySelector<HTMLElement>("[data-update-status]")!;
const updateCheckButton = document.querySelector<HTMLButtonElement>("[data-update-check]")!;
const updateDownloadButton = document.querySelector<HTMLButtonElement>("[data-update-download]")!;
const updateInstallButton = document.querySelector<HTMLButtonElement>("[data-update-install]")!;
const updateNotes = document.querySelector<HTMLElement>("[data-update-notes]")!;
const updateNotesRow = document.querySelector<HTMLElement>("[data-update-notes-row]")!;
const updateAutoDownload = document.querySelector<HTMLButtonElement>("[data-update-auto-download]")!;
const autoStart = document.querySelector<HTMLButtonElement>("[data-autostart]")!;
const language = document.querySelector<HTMLSelectElement>("[data-language]")!;
const mcpCommandOutput = document.querySelector<HTMLElement>("[data-mcp-command]")!;
const skillDirectoryOutput = document.querySelector<HTMLElement>("[data-skill-directory]")!;
const pageDescription = document.querySelector<HTMLElement>("[data-page-description]")!;
const agentStrips = {
  mcp: document.querySelector<HTMLElement>('[data-agent-strip="mcp"]')!,
  skills: document.querySelector<HTMLElement>('[data-agent-strip="skills"]')!,
};
const integrationStatuses = document.querySelectorAll<HTMLElement>(".integration-status");
const views = new Map<string, WidgetView>();
const deletedViews = new Map<string, DeletedWidgetView>();
const windowApi = getCurrentWindow();
const pages: Record<Page, { group: MessageKey; title: MessageKey }> = {
  "local-widgets": { group: "widgetsGroup", title: "localWidgets" },
  "community-widgets": { group: "widgetsGroup", title: "communityWidgets" },
  integrations: { group: "integrations", title: "mcpAndSkills" },
  general: { group: "settings", title: "general" },
  about: { group: "settings", title: "about" },
};

let state: MainState | undefined;
let page: Page = "local-widgets";
let filter: WidgetFilter = "all";
let theme: Theme = storedTheme();
let locale: Locale = storedLocale();
let widgetSort: WidgetSortOrder = "newest";
let integrationInfo: IntegrationInfo | undefined;
let integrationRequested = false;
let integrationFailed = false;
let integrationBusy = false;
let integrationProblems: AgentProblem[] = [];
let integrationFeedback = "";
let pendingPurge: DeletedWidgetView | undefined;
let purgeTimer: number | undefined;
let previewRefreshActive = false;
let widgetsReadyForPreviews = false;

const previewRefresh = createPreviewRefresh(
  () => page === "local-widgets" && !document.hidden,
  refreshAllPreviews,
);
const mainStartup = createMainStartup(
  () => invoke<boolean>("main_painted"),
  () => {
    widgetsReadyForPreviews = true;
    syncPreviewRefresh(true);
  },
);

const filterControl = createSegmentedControl(filterControlRoot, (value) => {
  selectFilter(value as WidgetFilter);
});
const themeControl = createSegmentedControl(themeControlRoot, (value) => {
  theme = value as Theme;
  saveTheme(theme);
  renderPreferences();
});
widgetSortButton.prepend(createElement(ArrowUpDown, {
  width: 14,
  height: 14,
  "stroke-width": 1.8,
  "aria-hidden": "true",
}));

function t(key: MessageKey) {
  return translate(resolvedLocale(locale), key);
}

function applyLocale() {
  const resolved = resolvedLocale(locale);
  document.documentElement.lang = resolved === "zh" ? "zh-CN" : "en";
  document.querySelectorAll<HTMLElement>("[data-message]").forEach((element) => {
    const key = element.dataset.message as MessageKey;
    element.textContent = t(key);
  });
  document.querySelector<HTMLElement>("nav")!.setAttribute("aria-label", t("mainNavigation"));
  filterOptions.setAttribute("aria-label", t("widgetFilters"));
  document.querySelectorAll<HTMLButtonElement>("[data-window-action]").forEach((button) => {
    button.setAttribute("aria-label", t(button.dataset.windowAction as MessageKey));
  });
  document.title = "aidget";
  if (state) render(state);
  renderPage();
}

function renderPage() {
  document.querySelectorAll<HTMLButtonElement>("[data-page]").forEach((button) => {
    const selected = button.dataset.page === page;
    button.classList.toggle("is-active", selected);
    if (selected) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  });
  document.querySelectorAll<HTMLElement>("[data-view]").forEach((view) => {
    view.hidden = view.dataset.view !== page;
  });
  currentGroup.textContent = t(pages[page].group);
  currentPage.textContent = t(pages[page].title);
  renderFilters();
  renderEmptyState();
  if (page === "integrations") ensureIntegrationInfo();
  if (page === "about") {
    refreshUpdateState();
    refreshAutoDownload();
  }
  renderIntegration();
  renderDescription();
  filterControl.refresh();
  themeControl.refresh();
}

function renderDescription() {
  if (page !== "integrations") return;
  renderMiddleEllipsis(pageDescription, t("mcpAndSkillsDescription"), 2);
}

function renderIntegration() {
  if (!integrationInfo) {
    const message = t(integrationFailed ? "integrationLoadFailed" : "loading");
    mcpCommandOutput.textContent = message;
    skillDirectoryOutput.textContent = message;
    agentStrips.mcp.replaceChildren();
    agentStrips.skills.replaceChildren();
    renderIntegrationStatus();
    return;
  }
  mcpCommandOutput.textContent = mcpCommandLine(integrationInfo);
  skillDirectoryOutput.textContent = integrationInfo.skillDirectory;
  for (const resource of ["mcp", "skills"] as const) {
    renderAgentStrip(
      agentStrips[resource],
      integrationInfo,
      resource,
      {
        all: t("agentAll"),
        empty: t("integrationNoAgents"),
        installFor: t("installFor"),
        removeFrom: t("removeFrom"),
      },
      integrationBusy,
      (agentIds, enabled) => void changeAgentIntegration(resource, agentIds, enabled),
    );
  }
  renderIntegrationStatus();
}

function ensureIntegrationInfo() {
  if (integrationInfo || integrationRequested) return;
  integrationRequested = true;
  void invoke<IntegrationInfo>("integration_state")
    .then((info) => {
      integrationInfo = info;
      integrationFailed = false;
      renderIntegration();
    })
    .catch((error: unknown) => {
      integrationFailed = true;
      renderIntegration();
      showFault(`${t("integrationLoadFailed")}: ${String(error)}`);
    });
}

function renderIntegrationStatus() {
  const message = integrationProblems.length
    ? `${t("integrationWriteFailed")}: ${integrationProblems[0].message}`
    : integrationFeedback;
  integrationStatuses.forEach((status) => {
    status.textContent = message;
    status.classList.toggle("is-error", integrationProblems.length > 0);
    status.hidden = message.length === 0;
  });
}

async function changeAgentIntegration(
  resource: IntegrationResource,
  agentIds: string[],
  enabled: boolean,
) {
  if (integrationBusy || agentIds.length === 0) return;
  integrationBusy = true;
  integrationProblems = [];
  integrationFeedback = "";
  renderIntegration();
  fault.hidden = true;
  try {
    const result = await invoke<IntegrationMutation>("set_agent_integration", {
      change: { agentIds, resource, enabled },
    });
    integrationInfo = result.state;
    integrationProblems = result.problems;
    integrationFeedback = result.problems.length ? "" : t("integrationRestart");
  } catch (error) {
    showFault(`${t("integrationWriteFailed")}: ${String(error)}`);
  } finally {
    integrationBusy = false;
    renderIntegration();
  }
}

function renderFilters() {
  filterControl.setValue(filter === "deleted" ? null : filter);
  const deleted = filter === "deleted";
  recycleFilter.classList.toggle("is-active", deleted);
  recycleFilter.setAttribute("aria-pressed", String(deleted));
  widgetSortLabel.textContent = t(widgetSort === "newest" ? "sortNewest" : "sortOldest");
  widgetSortButton.setAttribute(
    "aria-label",
    t(widgetSort === "newest" ? "sortNewestDescription" : "sortOldestDescription"),
  );
  widgetSortButton.title = widgetSortButton.getAttribute("aria-label") ?? "";
}

function selectFilter(next: WidgetFilter) {
  resetPurgeConfirmation();
  filter = next;
  if (state) render(state);
  else renderPage();
}

function renderEmptyState() {
  if (!state) return;
  if (filter === "deleted") {
    emptyTitle.textContent = t("noDeletedWidgets");
    emptyDescription.textContent = t("noDeletedWidgetsDescription");
    return;
  }
  const noWidgets = isEmpty(state);
  emptyTitle.textContent = t(noWidgets ? "noWidgets" : "noMatchingWidgets");
  emptyDescription.textContent = t(
    noWidgets ? "noWidgetsDescription" : "noMatchingWidgetsDescription",
  );
}

function renderPreferences() {
  themeControl.setValue(theme);
  language.value = locale;
}

// What the backend reports about the update flow.
interface UpdateSnapshot {
  state:
    | "idle"
    | "checking"
    | "latest"
    | "no_release"
    | "source"
    | "found"
    | "downloading"
    | "ready"
    | "installing"
    | "error";
  current: string;
  latest?: string;
  error?: string;
  done?: number;
  total?: number;
  notes?: string;
  staged: boolean;
}

let updateSnapshot: UpdateSnapshot | undefined;

const updateStateMessages: Record<UpdateSnapshot["state"], MessageKey> = {
  idle: "updateIdle",
  checking: "updateChecking",
  latest: "updateLatest",
  no_release: "updateNoRelease",
  source: "updateSource",
  found: "updateFound",
  downloading: "updateDownloading",
  ready: "updateReady",
  installing: "updateInstalling",
  error: "updateFailed",
};

function renderUpdate() {
  const snapshot = updateSnapshot;
  // What is possible comes from what is on disk, not from what the last network
  // step did: a failed check, download or install must not take away work that
  // is still available.
  const staged = snapshot?.staged ?? false;
  const busy =
    snapshot?.state === "checking" ||
    snapshot?.state === "downloading" ||
    snapshot?.state === "installing";
  const downloadable =
    !busy &&
    !staged &&
    Boolean(snapshot?.latest) &&
    snapshot?.state !== "idle" &&
    snapshot?.state !== "latest" &&
    snapshot?.state !== "no_release" &&
    snapshot?.state !== "source";
  updateDownloadButton.hidden = !downloadable;
  updateInstallButton.hidden = !staged || busy;
  // The notes belong to the update: an up-to-date build has nothing to say.
  const announced = snapshot?.state === "found" || snapshot?.state === "ready";
  const notes = announced ? snapshot?.notes : undefined;
  updateNotesRow.hidden = !notes;
  updateNotes.textContent = notes ?? "";
  if (!snapshot) {
    updateStatus.textContent = "—";
    return;
  }
  const { state, latest, error, done, total } = snapshot;
  const label = t(updateStateMessages[state]);
  const percent = state === "downloading" && total
    ? `${Math.floor(((done ?? 0) / total) * 100)}%`
    : undefined;
  const detail = state === "found" || state === "ready"
    ? latest
    : state === "error"
      ? error
      : percent;
  updateStatus.textContent = detail ? `${label} ${detail}` : label;
  // A failed check next to an update that is already downloaded: keep the
  // update and say why the check failed, instead of replacing one with the
  // other.
  if (state === "ready" && error) showFault(error);
}

function refreshAutoDownload() {
  void invoke<boolean>("update_auto_download")
    .then((enabled) => updateAutoDownload.setAttribute("aria-checked", String(enabled)))
    .catch((error: unknown) => {
      showFault(`${t("updateSettingsFailed")}: ${String(error)}`);
    });
}

let updatePoll: number | undefined;

// Re-read the snapshot while a step runs, so progress and the settle happen
// without the user pressing anything.
function watchUpdate() {
  window.clearTimeout(updatePoll);
  if (!updateSnapshot) return;
  const busy = updateSnapshot.state === "checking" || updateSnapshot.state === "downloading";
  if (!busy) return;
  updatePoll = window.setTimeout(() => {
    void invoke<UpdateSnapshot>("update_state")
      .then((snapshot) => {
        updateSnapshot = snapshot;
        renderUpdate();
        watchUpdate();
      })
      .catch(() => undefined);
  }, 400);
}

function refreshUpdateState() {
  void invoke<UpdateSnapshot>("update_state")
    .then((snapshot) => {
      updateSnapshot = snapshot;
      renderUpdate();
      watchUpdate();
    })
    .catch((error: unknown) => {
      showFault(`${t("updateFailed")}: ${String(error)}`);
    });
}

updateDownloadButton.addEventListener("click", async () => {
  updateDownloadButton.disabled = true;
  fault.hidden = true;
  try {
    updateSnapshot = await invoke<UpdateSnapshot>("update_download");
    renderUpdate();
    watchUpdate();
  } catch (error) {
    showFault(`${t("updateFailed")}: ${String(error)}`);
  } finally {
    updateDownloadButton.disabled = false;
  }
});

updateInstallButton.addEventListener("click", async () => {
  updateInstallButton.disabled = true;
  fault.hidden = true;
  try {
    // A successful install exits the app, so this call may never resolve.
    updateSnapshot = await invoke<UpdateSnapshot>("update_install");
    renderUpdate();
  } catch (error) {
    showFault(`${t("updateFailed")}: ${String(error)}`);
  } finally {
    updateInstallButton.disabled = false;
  }
});

updateAutoDownload.addEventListener("click", async () => {
  const enabled = updateAutoDownload.getAttribute("aria-checked") !== "true";
  updateAutoDownload.disabled = true;
  fault.hidden = true;
  try {
    await invoke("update_set_auto_download", { enabled });
    updateAutoDownload.setAttribute("aria-checked", String(enabled));
  } catch (error) {
    showFault(`${t("updateSettingsFailed")}: ${String(error)}`);
  } finally {
    updateAutoDownload.disabled = false;
  }
});

updateCheckButton.addEventListener("click", async () => {
  updateCheckButton.disabled = true;
  fault.hidden = true;
  try {
    updateSnapshot = await invoke<UpdateSnapshot>("update_check");
    renderUpdate();
    watchUpdate();
  } catch (error) {
    showFault(`${t("updateFailed")}: ${String(error)}`);
  } finally {
    updateCheckButton.disabled = false;
  }
});

function createView(widget: WidgetSummary): WidgetView {
  const root = document.createElement("article");
  root.className = "widget-card";
  root.dataset.widget = widget.id;

  const stage = document.createElement("div");
  stage.className = "preview-stage";
  const image = document.createElement("img");
  image.className = "preview-image";
  image.alt = "";
  image.width = Math.round(widget.width);
  image.height = Math.round(widget.height);
  image.hidden = true;
  const placeholder = document.createElement("span");
  placeholder.className = "preview-placeholder";
  placeholder.textContent = t("previewUnavailable");
  stage.append(image, placeholder);

  const info = document.createElement("div");
  info.className = "widget-info";
  const copy = document.createElement("div");
  copy.className = "widget-copy";
  const title = document.createElement("h2");
  title.className = "widget-title";
  const size = document.createElement("p");
  size.className = "widget-size";
  copy.append(title, size);

  const actions = document.createElement("div");
  actions.className = "widget-actions";
  const toggle = document.createElement("button");
  toggle.className = "switch";
  toggle.type = "button";
  toggle.setAttribute("role", "switch");
  toggle.addEventListener("click", () => void toggleWidget(view));
  const thumb = document.createElement("span");
  toggle.append(thumb);
  const archiveButton = document.createElement("button");
  archiveButton.className = "widget-icon-button archive-button";
  archiveButton.type = "button";
  archiveButton.append(createElement(Archive, {
    width: 16,
    height: 16,
    "stroke-width": 1.8,
    "aria-hidden": "true",
  }));
  archiveButton.addEventListener("click", () => void archiveWidget(view));
  actions.append(toggle, archiveButton);
  info.append(copy, actions);
  root.append(stage, info);

  const view = { root, image, placeholder, title, size, toggle, archiveButton, widget };
  updateView(view, widget);
  return view;
}

function updateView(view: WidgetView, widget: WidgetSummary) {
  view.placeholder.textContent = t("previewUnavailable");
  view.title.textContent = widget.title || widget.id;
  view.size.textContent = `${Math.round(widget.width)} × ${Math.round(widget.height)}`;
  view.image.width = Math.round(widget.width);
  view.image.height = Math.round(widget.height);
  view.toggle.setAttribute("aria-checked", String(widget.enabled));
  view.toggle.setAttribute("aria-label", t(widget.enabled ? "disable" : "enable"));
  view.archiveButton.setAttribute("aria-label", `${t("archive")} ${widget.title || widget.id}`);
  view.archiveButton.title = t("archive");
  view.root.dataset.enabled = String(widget.enabled);
  view.widget = widget;
}

function createDeletedView(widget: DeletedWidgetSummary): DeletedWidgetView {
  const root = document.createElement("article");
  root.className = "widget-card deleted-card";
  root.dataset.deleted = widget.trashName;

  const stage = document.createElement("div");
  stage.className = "preview-stage";
  const image = document.createElement("img");
  image.className = "preview-image";
  image.alt = "";
  image.width = Math.round(widget.width);
  image.height = Math.round(widget.height);
  image.hidden = true;
  const placeholder = document.createElement("span");
  placeholder.className = "preview-placeholder";
  placeholder.textContent = t("previewUnavailable");
  stage.append(image, placeholder);

  const info = document.createElement("div");
  info.className = "widget-info";
  const copy = document.createElement("div");
  copy.className = "widget-copy";
  const title = document.createElement("h2");
  title.className = "widget-title";
  const size = document.createElement("p");
  size.className = "widget-size";
  copy.append(title, size);

  const actions = document.createElement("div");
  actions.className = "widget-actions";
  const restoreButton = document.createElement("button");
  restoreButton.className = "widget-icon-button restore-button";
  restoreButton.type = "button";
  restoreButton.append(createElement(RotateCcw, {
    width: 16,
    height: 16,
    "stroke-width": 1.8,
    "aria-hidden": "true",
  }));
  const deleteButton = document.createElement("button");
  deleteButton.className = "widget-icon-button purge-button";
  deleteButton.type = "button";
  deleteButton.append(createElement(Trash, {
    width: 16,
    height: 16,
    "stroke-width": 1.8,
    "aria-hidden": "true",
  }));
  const confirmLabel = document.createElement("span");
  confirmLabel.className = "purge-confirm-label";
  deleteButton.append(confirmLabel);
  actions.append(restoreButton, deleteButton);
  info.append(copy, actions);
  root.append(stage, info);

  const view = {
    root,
    image,
    placeholder,
    title,
    size,
    restoreButton,
    deleteButton,
    trashName: widget.trashName,
  };
  restoreButton.addEventListener("click", () => void restoreDeletedWidget(view));
  deleteButton.addEventListener("click", () => armPurgeDeletedWidget(view));
  updateDeletedView(view, widget);
  if (widget.hasPreview) void loadDeletedPreview(view, widget);
  return view;
}

function updateDeletedView(view: DeletedWidgetView, widget: DeletedWidgetSummary) {
  view.placeholder.textContent = t("previewUnavailable");
  view.title.textContent = widget.title || widget.id;
  view.size.textContent = `${Math.round(widget.width)} × ${Math.round(widget.height)}`;
  view.restoreButton.setAttribute("aria-label", `${t("restore")} ${widget.title || widget.id}`);
  view.restoreButton.title = t("restore");
  view.deleteButton.dataset.widgetTitle = widget.title || widget.id;
  view.deleteButton.querySelector<HTMLElement>(".purge-confirm-label")!.textContent =
    t("confirmDelete");
  view.trashName = widget.trashName;
  syncPurgeButton(view, pendingPurge === view);
}

function renderDeletedWidgets(widgets: DeletedWidgetSummary[]) {
  const sorted = sortedDeletedWidgets(widgets, widgetSort);
  const alive = new Set(sorted.map((widget) => widget.trashName));
  if (pendingPurge && !alive.has(pendingPurge.trashName)) resetPurgeConfirmation();
  for (const [trashName, view] of deletedViews) {
    if (alive.has(trashName)) continue;
    view.root.remove();
    deletedViews.delete(trashName);
  }
  for (const widget of sorted) {
    const view = deletedViews.get(widget.trashName) ?? createDeletedView(widget);
    deletedViews.set(widget.trashName, view);
    updateDeletedView(view, widget);
  }
  const ordered = sorted.map((widget) => deletedViews.get(widget.trashName)!.root);
  if (ordered.some((node, index) => node !== deletedGrid.children[index])) {
    deletedGrid.append(...ordered);
  }
}

async function loadDeletedPreview(view: DeletedWidgetView, widget: DeletedWidgetSummary) {
  try {
    const preview = await invoke<string | null>("deleted_widget_preview", {
      trashName: widget.trashName,
    });
    if (view.trashName !== widget.trashName) return;
    view.image.hidden = preview === null;
    view.placeholder.hidden = preview !== null;
    if (preview !== null) view.image.src = preview;
  } catch (error) {
    view.image.hidden = true;
    view.placeholder.hidden = false;
    showFault(`${t("previewUnavailable")}: ${widget.title || widget.id} (${String(error)})`);
  }
}

async function loadPreview(view: WidgetView, widget: WidgetSummary) {
  const key = previewKey(widget);
  try {
    const preview = await invoke<string | null>("main_widget_preview", { id: widget.id });
    if (view.widget.id !== widget.id || previewKey(view.widget) !== key) return;
    view.image.hidden = preview === null;
    view.placeholder.hidden = preview !== null;
    if (preview !== null) view.image.src = preview;
  } catch (error) {
    showFault(`${t("previewUnavailable")}: ${widget.title || widget.id} (${String(error)})`);
  }
}

async function loadCachedPreview(view: WidgetView, widget: WidgetSummary) {
  const key = previewKey(widget);
  try {
    const preview = await invoke<string | null>("main_widget_cached_preview", {
      id: widget.id,
    });
    if (view.widget.id !== widget.id || previewKey(view.widget) !== key) return;
    view.image.hidden = preview === null;
    view.placeholder.hidden = preview !== null;
    if (preview !== null) view.image.src = preview;
  } catch (error) {
    showFault(`${t("previewUnavailable")}: ${widget.title || widget.id} (${String(error)})`);
  }
}

async function refreshAllPreviews() {
  const current = state;
  if (!current || page !== "local-widgets" || document.hidden) return;
  const widgets = filteredWidgets(
    sortedActiveWidgets(current.widgets, widgetSort),
    "enabled",
  );
  for (const widget of widgets) {
    if (page !== "local-widgets" || document.hidden) return;
    const view = views.get(widget.id);
    if (view) await loadPreview(view, widget);
  }
}

function syncPreviewRefresh(immediate = false) {
  if (!widgetsReadyForPreviews) return;
  const active = page === "local-widgets" && !document.hidden;
  if (!active) {
    previewRefresh.stop();
    previewRefreshActive = false;
    return;
  }
  const resumed = immediate && !previewRefreshActive;
  previewRefresh.start();
  previewRefreshActive = true;
  if (resumed) void previewRefresh.refreshNow();
}

function render(next: MainState) {
  const initial = state === undefined;
  const previous = new Map([...views].map(([id, view]) => [id, view.widget]));
  const widgets = sortedActiveWidgets(next.widgets, widgetSort);
  const deletedMode = filter === "deleted";
  const visibleWidgets = deletedMode ? [] : filteredWidgets(widgets, filter);
  const visibleIds = new Set(visibleWidgets.map((widget) => widget.id));
  const alive = new Set(widgets.map((widget) => widget.id));
  for (const [id, view] of views) {
    if (alive.has(id)) continue;
    view.root.remove();
    views.delete(id);
  }
  for (const widget of widgets) {
    const view = views.get(widget.id) ?? createView(widget);
    views.set(widget.id, view);
    updateView(view, widget);
    view.root.hidden = !visibleIds.has(widget.id);
    if (!previous.has(widget.id) || previewNeedsRefresh(previous.get(widget.id), widget)) {
      if (initial) void loadCachedPreview(view, widget);
      else void loadPreview(view, widget);
    }
  }
  const ordered = widgets.map((widget) => views.get(widget.id)!.root);
  if (ordered.some((node, index) => node !== grid.children[index])) grid.append(...ordered);
  renderDeletedWidgets(next.deletedWidgets);
  state = next;
  version.textContent = next.version;
  empty.hidden = deletedMode
    ? next.deletedWidgets.length > 0
    : visibleWidgets.length > 0;
  grid.hidden = deletedMode || visibleWidgets.length === 0;
  deletedGrid.hidden = !deletedMode || next.deletedWidgets.length === 0;
  renderPage();
}

function showFault(message: string) {
  fault.textContent = message;
  fault.hidden = false;
}

async function refreshState() {
  try {
    render(await invoke<MainState>("main_state"));
  } catch (error) {
    showFault(`${t("loadFailed")}: ${String(error)}`);
  }
}

async function toggleWidget(view: WidgetView) {
  view.toggle.disabled = true;
  fault.hidden = true;
  try {
    await invoke("set_widget_enabled", {
      id: view.widget.id,
      enabled: !view.widget.enabled,
    });
  } catch (error) {
    showFault(`${t("settingFailed")}: ${String(error)}`);
  } finally {
    view.toggle.disabled = false;
  }
}

async function archiveWidget(view: WidgetView) {
  view.archiveButton.disabled = true;
  fault.hidden = true;
  try {
    await invoke("delete_widget", { id: view.widget.id });
  } catch (error) {
    showFault(`${t("settingFailed")}: ${String(error)}`);
    view.archiveButton.disabled = false;
  }
}

async function restoreDeletedWidget(view: DeletedWidgetView) {
  if (pendingPurge === view) resetPurgeConfirmation();
  view.restoreButton.disabled = true;
  fault.hidden = true;
  try {
    await invoke("restore_widget", { trashName: view.trashName });
  } catch (error) {
    showFault(`${t("restoreFailed")}: ${String(error)}`);
    view.restoreButton.disabled = false;
  }
}

function syncPurgeButton(view: DeletedWidgetView, pending: boolean) {
  const title = view.deleteButton.dataset.widgetTitle ?? "";
  view.deleteButton.dataset.confirm = String(pending);
  view.deleteButton.setAttribute(
    "aria-label",
    `${t(pending ? "confirmDelete" : "deletePermanently")} ${title}`,
  );
  view.deleteButton.title = t(pending ? "confirmDelete" : "deletePermanently");
}

function resetPurgeConfirmation() {
  if (purgeTimer !== undefined) window.clearTimeout(purgeTimer);
  purgeTimer = undefined;
  if (!pendingPurge) return;
  syncPurgeButton(pendingPurge, false);
  pendingPurge = undefined;
}

function armPurgeDeletedWidget(view: DeletedWidgetView) {
  if (view.deleteButton.disabled) return;
  if (pendingPurge === view) {
    void purgeDeletedWidget(view);
    return;
  }
  resetPurgeConfirmation();
  pendingPurge = view;
  syncPurgeButton(view, true);
  purgeTimer = window.setTimeout(resetPurgeConfirmation, 3000);
}

async function purgeDeletedWidget(view: DeletedWidgetView) {
  resetPurgeConfirmation();
  view.deleteButton.disabled = true;
  fault.hidden = true;
  try {
    await invoke("purge_deleted_widget", { trashName: view.trashName });
  } catch (error) {
    showFault(`${t("deleteFailed")}: ${String(error)}`);
    view.deleteButton.disabled = false;
  }
}

async function initializeAutostart() {
  autoStart.setAttribute("aria-checked", String(await isEnabled()));
}

autoStart.addEventListener("click", async () => {
  autoStart.disabled = true;
  fault.hidden = true;
  try {
    if (autoStart.getAttribute("aria-checked") === "true") await disable();
    else await enable();
    await initializeAutostart();
  } catch (error) {
    showFault(`${t("startupFailed")}: ${String(error)}`);
  } finally {
    autoStart.disabled = false;
  }
});

document.querySelectorAll<HTMLButtonElement>("[data-page]").forEach((button) => {
  button.addEventListener("click", () => {
    resetPurgeConfirmation();
    page = button.dataset.page as Page;
    if (page === "integrations") {
      integrationRequested = false;
      integrationFailed = false;
      integrationInfo = undefined;
      integrationProblems = [];
      integrationFeedback = "";
    }
    fault.hidden = true;
    renderPage();
    syncPreviewRefresh(true);
  });
});

recycleFilter.addEventListener("click", () => {
  selectFilter("deleted");
});

widgetSortButton.addEventListener("click", () => {
  resetPurgeConfirmation();
  widgetSort = widgetSort === "newest" ? "oldest" : "newest";
  if (state) render(state);
  else renderPage();
});

document.addEventListener("pointerdown", (event) => {
  if (!pendingPurge) return;
  if (pendingPurge.deleteButton.contains(event.target as Node)) return;
  resetPurgeConfirmation();
}, true);

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && pendingPurge) resetPurgeConfirmation();
});

window.addEventListener("resize", renderDescription);
document.addEventListener("visibilitychange", () => syncPreviewRefresh(true));

language.addEventListener("change", () => {
  locale = language.value as Locale;
  saveLocale(locale);
  applyLocale();
});

document.querySelectorAll<HTMLElement>("[data-drag-region]").forEach((region) => {
  region.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || (event.target as Element).closest("button")) return;
    void windowApi.startDragging();
  });
});

document.querySelectorAll<HTMLButtonElement>("[data-window-action]").forEach((button) => {
  button.addEventListener("click", () => {
    const action = button.dataset.windowAction;
    const command = action === "minimize"
      ? windowApi.minimize()
      : action === "maximize"
        ? windowApi.toggleMaximize()
        : windowApi.close();
    void command.catch((error: unknown) => showFault(String(error)));
  });
});

themeControlRoot.setAttribute("aria-label", t("theme"));
filterOptions.setAttribute("aria-label", t("widgetFilters"));
autoStart.setAttribute("aria-label", t("launchAtLogin"));
autoStart.setAttribute("aria-checked", "false");
document.body.dataset.platform = /Macintosh|Mac OS X/.test(navigator.userAgent)
  ? "macos"
  : /Windows/.test(navigator.userAgent)
    ? "windows"
    : "linux";

brandMark.src = brandIcon;
createIcons({
  icons: { Info, LayoutGrid, Minus, ServerCog, Settings, Square, UsersRound, X },
});
saveTheme(theme);
saveLocale(locale);
renderPreferences();
applyLocale();
void initializeAutostart().catch((error: unknown) => showFault(`${t("startupFailed")}: ${String(error)}`));
void listen<MainState>("aidget://main", (event) => render(event.payload));
void listen("aidget://widgets-ready", () => mainStartup.synchronized());

void (async () => {
  await refreshState();
  await mainStartup.painted();
})().catch((error: unknown) => {
  showFault(`${t("startupFailed")}: ${String(error)}`);
});
