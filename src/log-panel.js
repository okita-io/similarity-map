// In-app debug log panel: captures console output, unhandled errors,
// and similarity-map:log events from the Rust backend.
// Tauri 2 IPC via window.__TAURI__.event.listen

const MAX_ENTRIES = 1000;

const LEVELS = ["debug", "info", "warn", "error"];

export class LogPanel {
  /**
   * @param {HTMLElement} container - Element to render the log panel into
   */
  constructor(container) {
    this.container = container;
    this._entries = [];
    this._collapsed = true;
    this._minLevel = "info";
    this._unlisten = null;

    this._buildUI();
    this._attachListeners();
    this._patchConsole();
    this._installErrorHandlers();
    this._listenForBackendLogs();

    this.log("info", "ui", "Log panel initialized");
  }

  _buildUI() {
    this.container.innerHTML = `
      <div class="log-panel ${this._collapsed ? "log-panel-collapsed" : ""}" id="log-panel">
        <div class="log-panel-header" id="log-panel-header">
          <button class="log-panel-toggle" type="button" id="log-panel-toggle" aria-expanded="${!this._collapsed}">
            <span class="log-panel-chevron">▾</span>
            <span class="log-panel-title">Log</span>
            <span class="log-panel-counts" id="log-panel-counts"></span>
          </button>
          <div class="log-panel-controls">
            <label class="log-panel-level-label" for="log-panel-level">Level:</label>
            <select class="log-panel-level" id="log-panel-level">
              ${LEVELS.map(
                (lvl) =>
                  `<option value="${lvl}" ${lvl === this._minLevel ? "selected" : ""}>${lvl}</option>`,
              ).join("")}
            </select>
            <button class="log-panel-btn" type="button" id="log-panel-copy" title="Copy all entries">Copy</button>
            <button class="log-panel-btn" type="button" id="log-panel-clear" title="Clear log">Clear</button>
          </div>
        </div>
        <div class="log-panel-body" id="log-panel-body" role="log" aria-live="polite"></div>
      </div>
    `;

    this._els = {
      panel: this.container.querySelector("#log-panel"),
      toggle: this.container.querySelector("#log-panel-toggle"),
      counts: this.container.querySelector("#log-panel-counts"),
      level: this.container.querySelector("#log-panel-level"),
      copyBtn: this.container.querySelector("#log-panel-copy"),
      clearBtn: this.container.querySelector("#log-panel-clear"),
      body: this.container.querySelector("#log-panel-body"),
    };
  }

  _attachListeners() {
    this._els.toggle.addEventListener("click", () => {
      this.toggle();
    });
    this._els.level.addEventListener("change", () => {
      this._minLevel = this._els.level.value;
      this._render();
    });
    this._els.clearBtn.addEventListener("click", () => {
      this._entries = [];
      this._render();
    });
    this._els.copyBtn.addEventListener("click", async () => {
      const text = this._entries
        .map((e) => `[${e.timestamp}] ${e.level.toUpperCase()} [${e.source}] ${e.message}`)
        .join("\n");
      try {
        await navigator.clipboard.writeText(text);
        this._flashButton(this._els.copyBtn, "Copied");
      } catch (err) {
        this.log("error", "ui", `Copy failed: ${String(err)}`);
      }
    });
  }

  _patchConsole() {
    const orig = {
      log: console.log.bind(console),
      info: console.info ? console.info.bind(console) : console.log.bind(console),
      warn: console.warn.bind(console),
      error: console.error.bind(console),
      debug: console.debug ? console.debug.bind(console) : console.log.bind(console),
    };
    const self = this;

    const wrap = (level, fn) => (...args) => {
      fn(...args);
      try {
        const msg = args.map((a) => self._formatArg(a)).join(" ");
        self._addEntry(level, "console", msg);
      } catch {
        // ignore: never let logging break the page
      }
    };

    console.log = wrap("info", orig.log);
    console.info = wrap("info", orig.info);
    console.warn = wrap("warn", orig.warn);
    console.error = wrap("error", orig.error);
    console.debug = wrap("debug", orig.debug);
  }

  _installErrorHandlers() {
    window.addEventListener("error", (event) => {
      const detail = event.error
        ? `${event.message}\n${event.error.stack || ""}`
        : event.message;
      this._addEntry("error", "window", detail);
    });
    window.addEventListener("unhandledrejection", (event) => {
      const reason = event.reason;
      this._addEntry(
        "error",
        "promise",
        reason && reason.stack ? reason.stack : this._formatArg(reason),
      );
    });
  }

  async _listenForBackendLogs() {
    const listen = window.__TAURI__?.event?.listen;
    if (!listen) {
      this._addEntry("warn", "ui", "Tauri event API unavailable — backend logs disabled");
      return;
    }
    try {
      this._unlisten = await listen("similarity-map:log", (event) => {
        const payload = event.payload || {};
        const level = LEVELS.includes(payload.level) ? payload.level : "info";
        const source = payload.source || "backend";
        const message = payload.message ?? this._formatArg(payload);
        this._addEntry(level, source, message);
      });
    } catch (err) {
      this._addEntry("warn", "ui", `Failed to subscribe to backend logs: ${String(err)}`);
    }
  }

  /** Public log API. */
  log(level, source, message) {
    this._addEntry(level, source, message);
  }

  _addEntry(level, source, message) {
    const entry = {
      timestamp: new Date().toISOString().slice(11, 23),
      level,
      source,
      message: typeof message === "string" ? message : this._formatArg(message),
    };
    this._entries.push(entry);
    if (this._entries.length > MAX_ENTRIES) {
      this._entries.splice(0, this._entries.length - MAX_ENTRIES);
    }
    this._appendEntryRow(entry);
    this._updateCounts();
  }

  _shouldShow(level) {
    return LEVELS.indexOf(level) >= LEVELS.indexOf(this._minLevel);
  }

  _appendEntryRow(entry) {
    if (!this._shouldShow(entry.level)) return;
    const row = document.createElement("div");
    row.className = `log-entry log-level-${entry.level}`;
    row.innerHTML = `
      <span class="log-entry-ts">${entry.timestamp}</span>
      <span class="log-entry-level">${entry.level.toUpperCase()}</span>
      <span class="log-entry-source">[${this._escapeHtml(entry.source)}]</span>
      <span class="log-entry-msg"></span>
    `;
    row.querySelector(".log-entry-msg").textContent = entry.message;
    const body = this._els.body;
    const wasAtBottom = body.scrollHeight - body.scrollTop - body.clientHeight < 20;
    body.appendChild(row);
    if (wasAtBottom) {
      body.scrollTop = body.scrollHeight;
    }
  }

  _render() {
    this._els.body.innerHTML = "";
    for (const entry of this._entries) {
      this._appendEntryRow(entry);
    }
    this._updateCounts();
  }

  _updateCounts() {
    const counts = { debug: 0, info: 0, warn: 0, error: 0 };
    for (const e of this._entries) {
      if (counts[e.level] !== undefined) counts[e.level]++;
    }
    const parts = [];
    if (counts.error) parts.push(`${counts.error} error${counts.error === 1 ? "" : "s"}`);
    if (counts.warn) parts.push(`${counts.warn} warn`);
    parts.push(`${this._entries.length} total`);
    this._els.counts.textContent = parts.join(" · ");
  }

  toggle() {
    this._collapsed = !this._collapsed;
    this._els.panel.classList.toggle("log-panel-collapsed", this._collapsed);
    this._els.toggle.setAttribute("aria-expanded", String(!this._collapsed));
    if (!this._collapsed) {
      this._els.body.scrollTop = this._els.body.scrollHeight;
    }
  }

  expand() {
    if (this._collapsed) this.toggle();
  }

  _formatArg(value) {
    if (value === null) return "null";
    if (value === undefined) return "undefined";
    if (typeof value === "string") return value;
    if (value instanceof Error) {
      return `${value.name}: ${value.message}${value.stack ? `\n${value.stack}` : ""}`;
    }
    try {
      return JSON.stringify(value);
    } catch {
      return String(value);
    }
  }

  _escapeHtml(s) {
    const div = document.createElement("div");
    div.textContent = String(s);
    return div.innerHTML;
  }

  _flashButton(btn, label) {
    const orig = btn.textContent;
    btn.textContent = label;
    btn.disabled = true;
    setTimeout(() => {
      btn.textContent = orig;
      btn.disabled = false;
    }, 900);
  }

  destroy() {
    if (this._unlisten) {
      this._unlisten();
      this._unlisten = null;
    }
  }
}
