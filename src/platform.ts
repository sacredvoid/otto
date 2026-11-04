// Windows or Mac wording and looks. Imported first by every page.

export const isWindows = navigator.userAgent.includes("Windows");
export const FOLDER_APP = isWindows ? "Explorer" : "Finder";

document.documentElement.classList.toggle("windows", isWindows);
const style = document.createElement("style");
style.textContent = "html:not(.windows) .win-only, html.windows .mac-only { display: none !important; }";
document.head.append(style);
// <span data-pc="PC">Mac</span> reads "PC" on Windows.
if (isWindows) document.querySelectorAll<HTMLElement>("[data-pc]").forEach((e) => (e.textContent = e.dataset.pc ?? ""));
