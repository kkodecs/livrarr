import { Info } from "lucide-react";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";

const REPO_URL = "https://github.com/kkodecs/livrarr";

export default function AboutPage() {
  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">About Livrarr</h1>
      </PageToolbar>

      <PageContent>
        <div className="max-w-2xl space-y-6">
          <div className="flex items-start gap-3">
            <Info size={20} className="text-brand shrink-0 mt-0.5" />
            <div className="space-y-4 text-sm text-zinc-400 leading-relaxed">
              <p>
                Livrarr (from <em>livre</em>, French for "book") is a self-hosted
                ebook and audiobook library manager built for the Servarr
                ecosystem. It automates the entire workflow from
                searching for books to organizing tagged files in your library,
                working alongside tools like Prowlarr, qBittorrent, SABnzbd,
                Calibre-Web Automated, and Audiobookshelf.
              </p>
              <p>
                The project manages both ebooks and audiobooks in a single
                application — users search for works, not formats, and grab
                releases in whichever media type they want. It was born from
                studying why earlier efforts in this space struggled. A detailed
                post-mortem of Readarr's architecture informed every design
                decision — from the works-first data model (books, not authors,
                are the primary entity) to the multi-user isolation that's built
                in from day one rather than retrofitted.
              </p>
              <p>
                Livrarr is built entirely with AI-assisted development. Not a
                single line of code was written by hand. The backend is
                approximately 25,000 lines of Rust across 10 crates; the frontend
                is React/TypeScript. The entire codebase was generated through a
                rigorous pipeline: detailed specification, cross-family adversarial
                review (Claude, Gemini, and GPT each reviewing each other's blind
                spots), intermediate representation (typed Rust signatures that
                constrain the generation space), behavioral tests written before
                implementation, and Rust's own type system as a final reviewer.
              </p>
              <p>
                When code generation is cheap, the specification becomes the
                critical input — not the hand-written code. Rust was chosen for the
                same reason: runtime performance and compile-time safety matter
                more when the cost of writing code approaches zero.
              </p>
              <p>
                Our sincere thanks to the user community for your support and
                feedback. You make this project better.
              </p>
              <p className="italic">
                — The Livrarr Dev Team, April 11, 2026
              </p>
            </div>
          </div>

          <div className="border-t border-border pt-4">
            <a
              href={REPO_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="text-sm text-brand hover:text-brand-hover"
            >
              GitHub Repository
            </a>
          </div>
        </div>
      </PageContent>
    </>
  );
}
