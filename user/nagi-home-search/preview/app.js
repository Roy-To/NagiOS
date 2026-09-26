(() => {
  "use strict";

  const categoryKeys = [
    ["everything", "search.category.everything"],
    ["apps", "search.category.apps"],
    ["files", "search.category.files"],
    ["notes", "search.category.notes"],
    ["activity", "search.category.activity"],
    ["actions", "search.category.actions"],
    ["workspaces", "search.category.workspaces"],
  ];
  const state = {
    locale: "en-US",
    strings: {},
    home: null,
    page: location.hash.startsWith("#/search") ? "search" : "home",
    query: new URLSearchParams(location.hash.split("?")[1] || "").get("q") || "",
    requestId: Date.now() * 1000,
    activeRequestId: null,
    searchAbort: null,
    results: [],
    category: "everything",
    selectedIndex: -1,
    debounce: null,
    returnFocus: null,
  };

  const app = document.getElementById("app");
  const localeSelect = document.getElementById("locale-select");
  const banner = document.getElementById("preview-banner");
  const dialog = document.getElementById("action-dialog");
  const toast = document.getElementById("toast");
  const offlineFallback = {
    "home.load_error": "Home preview is unavailable.",
    "home.load_error_help": "Start the local host preview server and reload.",
    "home.local_preview_only": "Local host preview only. No Nagi or host user data is read.",
    "ui.language_load_error": "Could not load the selected language.",
  };
  const catalog = (key) => state.strings[key] || offlineFallback[key] || "";

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function icon(symbol, className = "search-icon") {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("class", className);
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "1.8");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    if (symbol === "search") {
      const circle = document.createElementNS(svg.namespaceURI, "circle");
      circle.setAttribute("cx", "10.8");
      circle.setAttribute("cy", "10.8");
      circle.setAttribute("r", "6.5");
      const line = document.createElementNS(svg.namespaceURI, "path");
      line.setAttribute("d", "m16 16 4.5 4.5");
      svg.append(circle, line);
    } else {
      const path = document.createElementNS(svg.namespaceURI, "path");
      path.setAttribute("d", "M4 10.5 12 4l8 6.5v9a1 1 0 0 1-1 1h-5v-6h-4v6H5a1 1 0 0 1-1-1z");
      svg.append(path);
    }
    return svg;
  }

  function navigate(page, query = state.query) {
    window.clearTimeout(state.debounce);
    cancelActiveSearch();
    state.page = page;
    state.query = query;
    state.category = "everything";
    state.selectedIndex = -1;
    const hash = page === "search" ? `#/search${query ? `?q=${encodeURIComponent(query)}` : ""}` : "#/";
    if (location.hash !== hash) location.hash = hash;
    render();
    if (page === "search") {
      const input = document.getElementById("search-input");
      input?.focus();
    } else {
      document.getElementById("home-search-input")?.focus();
    }
  }

  function updateNavigation() {
    document.getElementById("nav-home").classList.toggle("active", state.page === "home");
    document.getElementById("nav-search").classList.toggle("active", state.page === "search");
  }

  function applyStaticLocalization() {
    document.title = `Nagi · ${catalog("home.title")} + ${catalog("search.title")}`;
    document.querySelector(".top-nav").setAttribute("aria-label", catalog("ui.nav_primary"));
    document.getElementById("nav-home").textContent = catalog("home.title");
    document.getElementById("nav-search").textContent = catalog("search.title");
    const languageLabel = catalog("ui.preview_language_label");
    document.querySelector(".locale-control .visually-hidden").textContent = languageLabel;
    localeSelect.setAttribute("aria-label", languageLabel);
    localeSelect.options[0].textContent = catalog("ui.language_english");
    localeSelect.options[1].textContent = catalog("ui.language_japanese");
    document.getElementById("dialog-close").setAttribute("aria-label", catalog("ui.close"));
    document.getElementById("dialog-done").textContent = catalog("ui.done");
    const loading = document.querySelector(".loading-screen");
    if (loading) loading.textContent = catalog("search.loading");
  }

  function searchField(id, placeholder, value, homeField = false) {
    const wrap = element("div", "search-wrap");
    wrap.append(icon("search"));
    const input = element("input", "search-field");
    input.id = id;
    input.type = "search";
    input.autocomplete = "off";
    input.spellcheck = false;
    input.setAttribute("aria-label", placeholder);
    input.setAttribute("role", "searchbox");
    input.setAttribute("aria-autocomplete", "list");
    input.setAttribute("aria-controls", homeField ? "home-search-results" : "results");
    input.placeholder = placeholder;
    input.value = value;
    const hint = element("span", "search-hint", catalog("search.keyboard_help"));
    wrap.append(input, hint);
    input.addEventListener("input", () => {
      state.query = input.value;
      cancelActiveSearch();
      state.results = [];
      state.providerIssues = [];
      state.selectedIndex = -1;
      window.clearTimeout(state.debounce);
      const target = homeField ? document.getElementById("home-search-results") : document.getElementById("results");
      if (homeField && !input.value.trim()) {
        renderResults(target);
        return;
      }
      renderLoading(target);
      state.debounce = window.setTimeout(() => runSearch(input.value, target), 120);
    });
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        window.clearTimeout(state.debounce);
        if (homeField && state.selectedIndex >= 0) activateSelectedResult();
        else if (homeField) navigate("search", input.value);
        else if (state.selectedIndex >= 0) activateSelectedResult();
        else runSearch(input.value, document.getElementById("results"));
      } else if (event.key === "Escape") {
        event.preventDefault();
        window.clearTimeout(state.debounce);
        if (!homeField) navigate("home", "");
        else {
          input.value = "";
          state.query = "";
          cancelActiveSearch();
          document.getElementById("home-search-results").replaceChildren();
        }
      } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        moveSelection(event.key === "ArrowDown" ? 1 : -1);
      }
    });
    return wrap;
  }

  function renderHome() {
    app.replaceChildren();
    const heading = element("div", "page-heading");
    const titleBox = element("div");
    titleBox.append(element("p", "eyebrow", catalog("home.eyebrow")));
    titleBox.append(element("h1", "", catalog("home.title")));
    titleBox.append(element("p", "page-subtitle", catalog("home.subtitle")));
    heading.append(titleBox);
    app.append(heading);

    const search = searchField("home-search-input", catalog("home.search_placeholder"), state.query, true);
    search.classList.add("home-search");
    app.append(search);
    const suggestions = element("div", "home-suggestions", "");
    suggestions.id = "home-search-results";
    suggestions.setAttribute("role", "listbox");
    app.append(suggestions);

    const homeGrid = element("section", "home-grid");
    homeGrid.setAttribute("aria-label", catalog("home.workspace_section"));
    const workspace = state.home.currentWorkspace;
    const workspaceCard = element("article", "surface workspace-card");
    const workspaceTop = element("div", "workspace-top");
    workspaceTop.append(element("p", "workspace-label", catalog("home.workspace_section")));
    workspaceTop.append(element("span", "small-pill", workspace ? `${catalog("home.workspace_id")} · ${workspace.workspaceId}` : catalog("home.no_workspace")));
    workspaceCard.append(workspaceTop);
    if (workspace) {
      workspaceCard.append(element("h2", "workspace-title", workspace.title));
      const count = workspace.relatedObjectCount;
      workspaceCard.append(element("p", "workspace-meta", `${count} ${catalog("home.related_objects")}`));
      const bottom = element("div", "workspace-bottom");
      const open = element("button", "button button-primary", catalog("home.open_workspace"));
      open.type = "button";
      open.addEventListener("click", () => showAction(workspace.title, workspace.action, workspace.actionLabel));
      bottom.append(open);
      const badge = element("span", "availability host_preview", workspace.actionLabel);
      bottom.append(badge);
      workspaceCard.append(bottom);
    } else {
      workspaceCard.append(element("p", "workspace-meta", catalog("home.no_workspace")));
    }
    homeGrid.append(workspaceCard);

    const continueCard = element("section", "surface continue-card");
    const continueTitle = element("div", "section-title");
    continueTitle.append(element("h2", "", catalog("home.continue_section")));
    continueTitle.append(element("span", "section-kicker", catalog("home.workspace_references")));
    continueCard.append(continueTitle);
    const continueList = element("div", "continue-list");
    for (const item of state.home.continuations) {
      const row = element("div", "continue-row");
      row.append(element("span", "mini-icon", item.title.slice(0, 1)));
      const copy = element("div", "continue-copy");
      copy.append(element("strong", "", item.title));
      copy.append(element("span", "", item.subtitle || item.actionLabel));
      row.append(copy);
      const action = element("button", "text-button", catalog("home.show_action"));
      action.type = "button";
      action.addEventListener("click", () => showAction(item.title, item.action, item.actionLabel));
      row.append(action);
      continueList.append(row);
    }
    if (!state.home.continuations.length) continueList.append(element("p", "workspace-meta", catalog("home.no_continuations")));
    continueCard.append(continueList);
    homeGrid.append(continueCard);
    app.append(homeGrid);

    const section = element("section", "apps-section");
    const sectionTitle = element("div", "section-title");
    sectionTitle.append(element("h2", "", catalog("home.apps_section")));
    sectionTitle.append(element("span", "section-kicker", catalog("home.registry_label")));
    section.append(sectionTitle);
    const appsGrid = element("div", "apps-grid");
    for (const item of state.home.apps) {
      const card = element("article", "app-card");
      const appIcon = element("span", "app-icon", item.iconGlyph);
      appIcon.style.backgroundColor = item.accentColor;
      appIcon.setAttribute("aria-hidden", "true");
      card.append(appIcon);
      const copy = element("div", "app-copy");
      copy.append(element("strong", "", item.title));
      copy.append(element("p", "", item.description));
      copy.append(element("span", `availability ${item.actionAvailability}`, item.actionLabel));
      card.append(copy);
      const launch = element("button", "app-launch", item.actionAvailability === "host_preview" ? catalog("search.open_preview") : item.actionLabel);
      launch.type = "button";
      launch.disabled = !["host_preview", "available"].includes(item.actionAvailability);
      launch.setAttribute("aria-label", `${item.title}: ${item.actionLabel}`);
      launch.addEventListener("click", () => {
        if (item.actionAvailability !== "host_preview") return;
        if (item.previewRoute) location.hash = item.previewRoute;
        else showAction(item.title, item.action, item.actionLabel);
      });
      card.append(launch);
      appsGrid.append(card);
    }
    if (state.home.apps.length) section.append(appsGrid);
    else section.append(emptyState(catalog("home.no_apps"), ""));
    app.append(section);

    const intent = element("section", "intent-card");
    intent.append(element("p", "", catalog("home.intent_entry")));
    const intentButton = element("button", "button button-quiet", catalog("availability.coming_soon"));
    intentButton.type = "button";
    intentButton.addEventListener("click", () => showAction(catalog("home.intent_entry"), { type: "open_intent_entry" }, catalog("availability.coming_soon")));
    intent.append(intentButton);
    app.append(intent);
    updateNavigation();
    if (state.query.trim()) runSearch(state.query, document.getElementById("home-search-results"));
  }

  function renderSearch() {
    app.replaceChildren();
    const heading = element("div", "page-heading");
    const titleBox = element("div");
    titleBox.append(element("p", "eyebrow", catalog("search.eyebrow")));
    titleBox.append(element("h1", "", catalog("search.title")));
    titleBox.append(element("p", "page-subtitle", catalog("search.keyboard_help")));
    heading.append(titleBox);
    app.append(heading);
    app.append(searchField("search-input", catalog("search.search_placeholder"), state.query));
    const toolbar = element("div", "search-toolbar");
    const categories = element("div", "category-list");
    categories.setAttribute("role", "tablist");
    categories.setAttribute("aria-label", catalog("search.category_list_label"));
    for (const [key, labelKey] of categoryKeys) {
      const chip = element("button", `category-chip${state.category === key ? " active" : ""}`, catalog(labelKey));
      chip.type = "button";
      chip.setAttribute("role", "tab");
      chip.setAttribute("aria-selected", String(state.category === key));
      chip.addEventListener("click", () => {
        state.category = key;
        state.selectedIndex = -1;
        renderCategories();
        renderResults(document.getElementById("results"));
      });
      categories.append(chip);
    }
    categories.id = "categories";
    toolbar.append(categories);
    toolbar.append(element("span", "result-count", `${state.results.length} ${catalog("search.result_count")}`)).id = "result-count";
    app.append(toolbar);
    const results = element("div", "results-list");
    results.id = "results";
    results.setAttribute("role", "listbox");
    app.append(results);
    app.append(element("div", "provider-issues")).id = "provider-issues";
    updateNavigation();
    if (state.query.trim()) runSearch(state.query, results);
    else renderResults(results);
  }

  function renderCategories() {
    const old = document.getElementById("categories");
    if (!old) return;
    const selected = state.category;
    old.replaceChildren();
    for (const [key, labelKey] of categoryKeys) {
      const chip = element("button", `category-chip${selected === key ? " active" : ""}`, catalog(labelKey));
      chip.type = "button";
      chip.setAttribute("role", "tab");
      chip.setAttribute("aria-selected", String(selected === key));
      chip.addEventListener("click", () => {
        state.category = key;
        state.selectedIndex = -1;
        renderCategories();
        renderResults(document.getElementById("results"));
      });
      old.append(chip);
    }
  }

  function cancelActiveSearch() {
    if (state.searchAbort) state.searchAbort.abort();
    if (state.activeRequestId !== null) {
      fetch(`/api/cancel?request_id=${state.activeRequestId}`, { method: "POST" }).catch(() => {});
    }
    state.searchAbort = null;
    state.activeRequestId = null;
  }

  async function runSearch(query, target) {
    if (!target) return;
    if (!query.trim()) {
      state.results = [];
      state.query = "";
      renderResults(target);
      return;
    }
    if (state.searchAbort) state.searchAbort.abort();
    const previousRequestId = state.activeRequestId;
    if (previousRequestId !== null) {
      fetch(`/api/cancel?request_id=${previousRequestId}`, { method: "POST" }).catch(() => {});
    }
    const requestId = ++state.requestId;
    const abort = new AbortController();
    state.searchAbort = abort;
    state.activeRequestId = requestId;
    if (state.page === "search") renderLoading(target);
    try {
      const params = new URLSearchParams({ q: query, locale: state.locale, request_id: String(requestId) });
      const response = await fetch(`/api/search?${params}`, { signal: abort.signal, cache: "no-store" });
      if (response.status === 409) return;
      if (!response.ok) throw new Error("search_request_failed");
      const payload = await response.json();
      if (abort.signal.aborted || payload.requestId !== state.activeRequestId) return;
      state.results = payload.results;
      state.providerIssues = payload.providerIssues;
      state.strings = payload.strings;
      state.selectedIndex = -1;
      renderResults(target);
    } catch (error) {
      if (error.name === "AbortError") return;
      target.replaceChildren(emptyState(catalog("search.error"), ""));
    }
  }

  function renderLoading(target) {
    target.replaceChildren(element("div", "empty-state", catalog("search.loading")));
    const input = document.getElementById(state.page === "home" ? "home-search-input" : "search-input");
    input?.setAttribute("aria-expanded", "false");
    input?.removeAttribute("aria-activedescendant");
  }

  function renderResults(target) {
    if (!target) return;
    const visible = state.category === "everything" ? state.results : state.results.filter((item) => item.category === state.category);
    target.replaceChildren();
    const input = document.getElementById(state.page === "home" ? "home-search-input" : "search-input");
    input?.setAttribute("aria-expanded", "false");
    input?.removeAttribute("aria-activedescendant");
    if (state.page === "search") {
      const count = document.getElementById("result-count");
      if (count) count.textContent = `${visible.length} ${catalog("search.result_count")}`;
    }
    const issueTarget = document.getElementById("provider-issues");
    if (issueTarget) {
      issueTarget.replaceChildren();
      const messages = (state.providerIssues || []).map((issue) => {
        const key = issue.kind === "timed_out" ? "search.provider_timeout" :
          issue.kind === "permission_denied" ? "search.provider_denied" : "search.provider_failed";
        return `${issue.providerId}: ${catalog(key)}`;
      });
      if (messages.length) issueTarget.textContent = messages.join(" · ");
    }
    if (!state.query.trim()) {
      if (state.page === "search") target.append(emptyState(catalog("search.empty_prompt"), ""));
      return;
    }
    if (!visible.length) {
      target.append(emptyState(catalog("search.no_results"), ""));
      return;
    }
    visible.forEach((result, index) => target.append(resultCard(result, index)));
    input?.setAttribute("aria-expanded", "true");
    if (state.selectedIndex >= 0) {
      const selected = visible[state.selectedIndex];
      if (selected) input?.setAttribute("aria-activedescendant", `search-result-${selected.resultId}`);
    }
  }

  function emptyState(title, description) {
    const box = element("div", "empty-state");
    if (title) box.append(element("strong", "", title));
    if (description) box.append(element("p", "", description));
    return box;
  }

  function resultCard(result, index) {
    const card = element("article", `result-card${state.selectedIndex === index ? " selected" : ""}`);
    card.setAttribute("role", "option");
    card.setAttribute("aria-selected", String(state.selectedIndex === index));
    card.id = `search-result-${result.resultId}`;
    card.dataset.resultIndex = String(index);
    const symbol = result.category === "apps" ? "◈" : result.category === "workspaces" ? "⌂" : result.category === "activity" ? "◷" : result.category === "actions" ? "↗" : "▤";
    card.append(element("span", "result-symbol", symbol));
    const copy = element("div", "result-copy");
    const heading = element("div", "result-heading");
    heading.append(element("strong", "", result.title));
    heading.append(element("span", "category-label", result.categoryLabel));
    copy.append(heading);
    if (result.subtitle) copy.append(element("p", "result-subtitle", result.subtitle));
    const metadata = element("div", "result-meta");
    metadata.append(element("span", "", result.matchReasonLabel));
    metadata.append(element("span", "", result.isFixture ? catalog("search.source_fixture") : result.providerId));
    if (result.actionLabel) metadata.append(element("span", `availability ${result.actionAvailability}`, result.actionLabel));
    copy.append(metadata);
    card.append(copy);
    const action = element("button", "button button-quiet result-action", catalog("search.open_preview"));
    action.type = "button";
    action.dataset.actionIndex = String(index);
    action.addEventListener("click", () => showAction(result.title, result.action, result.actionLabel, result.preview));
    card.append(action);
    return card;
  }

  function visibleResults() {
    return state.category === "everything" ? state.results : state.results.filter((item) => item.category === state.category);
  }

  function moveSelection(delta) {
    const results = visibleResults();
    if (!results.length) return;
    state.selectedIndex = Math.max(0, Math.min(results.length - 1, state.selectedIndex + delta));
    const target = document.getElementById(state.page === "home" ? "home-search-results" : "results");
    if (target) renderResults(target);
    document.querySelector(`.result-card[data-result-index="${state.selectedIndex}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function activateSelectedResult() {
    const result = visibleResults()[state.selectedIndex];
    if (result) showAction(result.title, result.action, result.actionLabel, result.preview);
  }

  function showAction(title, action, availability, preview = "") {
    state.returnFocus = document.activeElement;
    document.getElementById("dialog-eyebrow").textContent = availability || catalog("search.preview_action_notice");
    document.getElementById("dialog-title").textContent = title;
    document.getElementById("dialog-description").textContent = catalog("search.preview_action_notice");
    document.getElementById("dialog-action").textContent = JSON.stringify({ action, preview: preview || undefined }, null, 2);
    dialog.showModal();
    document.getElementById("dialog-done").focus();
  }

  function closeDialog() {
    dialog.close();
    state.returnFocus?.focus?.();
  }

  function notify(message) {
    toast.textContent = message;
    toast.classList.add("visible");
    window.clearTimeout(notify.timeout);
    notify.timeout = window.setTimeout(() => toast.classList.remove("visible"), 2200);
  }

  async function loadHome() {
    const response = await fetch(`/api/home?locale=${encodeURIComponent(state.locale)}`, { cache: "no-store" });
    if (!response.ok) throw new Error("home_request_failed");
    const payload = await response.json();
    state.home = payload;
    state.strings = payload.strings;
    document.documentElement.lang = payload.locale;
    banner.textContent = catalog("home.preview_notice");
    localeSelect.value = payload.locale;
  }

  function render() {
    if (!state.home) return;
    banner.textContent = catalog("home.preview_notice");
    if (state.page === "search") renderSearch();
    else renderHome();
  }

  async function refreshLocale() {
    state.locale = localeSelect.value;
    await loadHome();
    applyStaticLocalization();
    render();
  }

  localeSelect.addEventListener("change", () => refreshLocale().catch(() => notify(catalog("ui.language_load_error"))));
  document.getElementById("dialog-close").addEventListener("click", closeDialog);
  document.getElementById("dialog-done").addEventListener("click", closeDialog);
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) closeDialog();
  });
  window.addEventListener("hashchange", () => {
    const page = location.hash.startsWith("#/search") ? "search" : "home";
    if (page === state.page) return;
    window.clearTimeout(state.debounce);
    cancelActiveSearch();
    state.page = page;
    state.category = "everything";
    state.selectedIndex = -1;
    if (page === "search") {
      const query = new URLSearchParams(location.hash.split("?")[1] || "").get("q") || "";
      state.query = query;
    }
    render();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && dialog.open) closeDialog();
  });

  loadHome()
    .then(() => {
      applyStaticLocalization();
      render();
    })
    .catch(() => {
      app.replaceChildren(emptyState(catalog("home.load_error"), catalog("home.load_error_help")));
      banner.textContent = catalog("home.local_preview_only");
    });
})();
