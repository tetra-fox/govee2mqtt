export type ThemeMode = "system" | "dark" | "light";

function read(): ThemeMode {
  const v = localStorage.getItem("theme");
  return v === "dark" || v === "light" ? v : "system";
}

class Theme {
  mode = $state<ThemeMode>(read());

  #mql = matchMedia("(prefers-color-scheme: dark)");
  #onSystemChange = () => this.#apply();

  constructor() {
    this.#apply();
    this.#mql.addEventListener("change", this.#onSystemChange);
  }

  set(mode: ThemeMode) {
    this.mode = mode;
    localStorage.setItem("theme", mode);
    this.#apply();
  }

  #apply() {
    const dark = this.mode === "dark" || (this.mode === "system" && this.#mql.matches);
    document.documentElement.classList.toggle("dark", dark);
  }
}

export const theme = new Theme();
