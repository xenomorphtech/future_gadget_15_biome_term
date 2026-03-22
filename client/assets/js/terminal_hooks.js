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
    this.el.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && e.ctrlKey) {
        e.preventDefault();
        this.el.form?.requestSubmit();
      }
    });
  },
};
