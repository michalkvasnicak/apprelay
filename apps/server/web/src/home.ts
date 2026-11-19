import { trackPageview } from "./analytics";

trackPageview();

const copyButton = document.querySelector<HTMLButtonElement>("#copy");
copyButton?.addEventListener("click", () => {
  void navigator.clipboard?.writeText("npx @brosoft/apprelay app.ipa").then(() => {
    copyButton.textContent = "Copied";
    setTimeout(() => {
      copyButton.textContent = "Copy";
    }, 1600);
  });
});
