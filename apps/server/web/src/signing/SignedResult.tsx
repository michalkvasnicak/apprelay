import { useEffect, useRef } from "react";

import { buttonClass, secondaryClass } from "../components/forms";
import { signingController } from "./controller";

export function SignedResult() {
  const download = useRef<HTMLAnchorElement>(null);

  useEffect(() => {
    const link = download.current;
    if (!link) {
      return;
    }
    const file = signingController.signedFile();
    const url = URL.createObjectURL(file);
    link.href = url;
    link.download = file.name;
    return () => {
      link.removeAttribute("href");
      URL.revokeObjectURL(url);
    };
  }, []);

  return (
    <section aria-labelledby="signed-title">
      <h2 id="signed-title" className="text-lg font-semibold">
        Your app is signed
      </h2>
      <p role="status" className="mt-3 text-sm leading-6 text-muted">
        Download the signed IPA to validate it. Nothing has been sent to your iPhone. Installation
        is not available yet.
      </p>
      <div className="mt-5 flex flex-wrap items-center gap-5">
        <a ref={download} className={buttonClass}>
          Download signed IPA
        </a>
        <a href="/" className={secondaryClass}>
          Sign another app
        </a>
      </div>
      <p className="mt-4 text-xs leading-5 text-muted">
        The signed IPA is kept only in memory and is discarded when you leave or reload this page.
      </p>
    </section>
  );
}
