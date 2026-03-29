const SNIPPET_HISTORY_LIMIT = 50;

export const TerminalInput = {
  mounted() {
    this.el.addEventListener("keydown", (e) => {
      e.preventDefault();
      const key =
        e.key === "Enter"
          ? "\r"
          : e.key === "Backspace"
          ? "\x7f"
          : e.key === "Tab"
          ? "\t"
          : e.key === "Escape"
          ? "\x1b"
          : e.key === "ArrowUp"
          ? "\x1b[A"
          : e.key === "ArrowDown"
          ? "\x1b[B"
          : e.key === "ArrowRight"
          ? "\x1b[C"
          : e.key === "ArrowLeft"
          ? "\x1b[D"
          : e.ctrlKey && e.key.length === 1
          ? String.fromCharCode(e.key.charCodeAt(0) & 0x1f)
          : e.key.length === 1
          ? e.key
          : null;

      if (key) this.pushEvent("send_input", { key });
    });
  },
};

export const SnippetInput = {
  mounted() {
    this.historyCursor = null;
    this.draftValue = "";
    this.historyScope = this.currentHistoryScope();

    this.handleKeyDown = (e) => {
      const textarea = this.getTextarea();

      if (e.target !== textarea) {
        return;
      }

      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        const value = textarea.value;
        if (value.trim() !== "") {
          this.pushEvent("inject_snippet", { snippet: value });
          this.saveHistoryEntry(value);
          this.resetNavigation();
          textarea.value = "";
          textarea.dispatchEvent(new Event("input", { bubbles: true }));
        }
        return;
      }

      if (e.altKey && e.key === "ArrowUp") {
        e.preventDefault();
        this.navigateHistory(-1);
        return;
      }

      if (e.altKey && e.key === "ArrowDown") {
        e.preventDefault();
        this.navigateHistory(1);
      }
    };

    this.handleSubmit = () => {
      const textarea = this.getTextarea();
      const value = textarea?.value ?? "";

      if (this.historyScope == "" || value.trim() == "") {
        return;
      }

      this.saveHistoryEntry(value);
      this.resetNavigation();
      this.syncControls();
    };

    this.handleClick = (e) => {
      const button = e.target.closest("[data-history-nav]");

      if (!button || button.disabled) {
        return;
      }

      const direction = button.dataset.historyNav === "prev" ? -1 : 1;
      this.navigateHistory(direction);
    };

    this.el.addEventListener("keydown", this.handleKeyDown);
    this.el.addEventListener("submit", this.handleSubmit);
    this.el.addEventListener("click", this.handleClick);
    this.syncControls();
  },

  updated() {
    const nextScope = this.currentHistoryScope();

    if (nextScope !== this.historyScope) {
      this.historyScope = nextScope;
      this.resetNavigation();
    }

    this.syncControls();
  },

  destroyed() {
    this.el.removeEventListener("keydown", this.handleKeyDown);
    this.el.removeEventListener("submit", this.handleSubmit);
    this.el.removeEventListener("click", this.handleClick);
  },

  currentHistoryScope() {
    return this.el.dataset.historyScope ?? "";
  },

  getTextarea() {
    return this.el.querySelector("textarea[name='snippet']");
  },

  getHistoryStorageKey() {
    return `snippet-history:${this.historyScope}`;
  },

  getHistory() {
    if (this.historyScope == "") {
      return [];
    }

    try {
      const raw = window.sessionStorage.getItem(this.getHistoryStorageKey());
      const history = raw ? JSON.parse(raw) : [];

      return Array.isArray(history) ? history.filter((entry) => typeof entry === "string") : [];
    } catch (_error) {
      return [];
    }
  },

  saveHistory(history) {
    if (this.historyScope == "") {
      return;
    }

    try {
      window.sessionStorage.setItem(
        this.getHistoryStorageKey(),
        JSON.stringify(history.slice(-SNIPPET_HISTORY_LIMIT)),
      );
    } catch (_error) {
      // Ignore storage failures and keep the input usable.
    }
  },

  saveHistoryEntry(entry) {
    const history = this.getHistory();

    if (history[history.length - 1] === entry) {
      return;
    }

    history.push(entry);
    this.saveHistory(history);
  },

  resetNavigation() {
    this.historyCursor = null;
    this.draftValue = "";
  },

  navigateHistory(direction) {
    const textarea = this.getTextarea();
    const history = this.getHistory();

    if (!textarea || history.length === 0) {
      return;
    }

    if (direction < 0) {
      if (this.historyCursor === null) {
        this.draftValue = textarea.value;
        this.historyCursor = history.length - 1;
      } else if (this.historyCursor > 0) {
        this.historyCursor -= 1;
      } else {
        return;
      }

      this.applySnippetValue(history[this.historyCursor]);
      return;
    }

    if (this.historyCursor === null) {
      return;
    }

    if (this.historyCursor < history.length - 1) {
      this.historyCursor += 1;
      this.applySnippetValue(history[this.historyCursor]);
    } else {
      this.historyCursor = null;
      this.applySnippetValue(this.draftValue);
    }
  },

  applySnippetValue(value) {
    const textarea = this.getTextarea();

    if (!textarea) {
      return;
    }

    textarea.value = value;
    textarea.dispatchEvent(new Event("input", {bubbles: true}));
    textarea.focus();
    textarea.selectionStart = textarea.value.length;
    textarea.selectionEnd = textarea.value.length;
    this.syncControls();
  },

  syncControls() {
    const prevButton = this.el.querySelector("[data-history-nav='prev']");
    const nextButton = this.el.querySelector("[data-history-nav='next']");
    const history = this.getHistory();
    const hasScope = this.historyScope !== "";
    const hasHistory = hasScope && history.length > 0;

    if (prevButton) {
      prevButton.disabled = !hasHistory || this.historyCursor === 0;
    }

    if (nextButton) {
      nextButton.disabled = !hasHistory || this.historyCursor === null;
    }
  },
};
