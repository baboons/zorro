#!/usr/bin/env bash
# Create a throwaway repo with several conflicting files across a merge, then
# open Zorro on it. Includes a longer file (Book.java) with additions AND
# removals on both sides, for testing the red/green line diff.
set -euo pipefail

DEMO="${1:-/tmp/zorro-demo}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

rm -rf "$DEMO"
mkdir -p "$DEMO"
cd "$DEMO"

git init -q -b main
git config user.email demo@zorro.dev
git config user.name "Zorro Demo"
git config commit.gpgsign false
# diff3 markers include the common ancestor (the |||||||  base section).
git config merge.conflictStyle diff3

# ---------------------------------------------------------------- base --------
cat > Book.java <<'EOF'
package library;

import java.util.List;

public class Book {

    private final String title;
    private final List<String> authors;
    private int pages;

    public Book(String title, List<String> authors) {
        this.title = title;
        this.authors = authors;
    }

    public String title() {
        return title;
    }

    public boolean containsAuthor(String author) {
        return authors.contains(author);
    }

    public int pageCount() {
        return pages;
    }

    public String summary() {
        return title + " by " + authors;
    }
}
EOF

cat > config.json <<'EOF'
{
  "name": "service",
  "port": 8080,
  "logLevel": "info"
}
EOF

cat > settings.py <<'EOF'
TIMEOUT = 30
RETRIES = 3
DEBUG = False
EOF

git add -A && git commit -q -m base

# ------------------------------------------------------------- feature --------
git checkout -q -b feature
cat > Book.java <<'EOF'
package library;

import java.util.List;
import java.util.Set;

public class Book {

    private final String title;
    private final List<String> authors;
    private int pages;

    public Book(String title, List<String> authors) {
        this.title = title;
        this.authors = authors;
    }

    public String title() {
        return title;
    }

    public boolean containsAuthor(String author) {
        return Set.copyOf(authors).contains(author);
    }

    public int pageCount() {
        return pages;
    }

    public String summary() {
        return "\"" + title + "\" — " + authors.size() + " authors";
    }
}
EOF
cat > config.json <<'EOF'
{
  "name": "service",
  "port": 9090,
  "logLevel": "debug"
}
EOF
cat > settings.py <<'EOF'
TIMEOUT = 90
RETRIES = 5
DEBUG = False
EOF
git commit -q -am "feature changes"

# ---------------------------------------------------------------- main --------
git checkout -q main
cat > Book.java <<'EOF'
package library;

import java.util.List;
import java.util.Objects;

public class Book {

    private final String title;
    private final List<String> authors;
    private int pages;

    public Book(String title, List<String> authors) {
        this.title = title;
        this.authors = authors;
    }

    public String title() {
        return title;
    }

    public boolean containsAuthor(String author) {
        return authors.stream().anyMatch(a -> a.equalsIgnoreCase(author));
    }

    public int pageCount() {
        return pages;
    }

    public String summary() {
        return title + " (" + pages + " pages)";
    }
}
EOF
cat > config.json <<'EOF'
{
  "name": "service",
  "port": 8080,
  "logLevel": "warn"
}
EOF
cat > settings.py <<'EOF'
TIMEOUT = 60
RETRIES = 3
DEBUG = True
EOF
git commit -q -am "main changes"

# --------------------------------------------------------- trigger conflict ---
git merge feature || true

echo "Demo repo ready at $DEMO"
echo "Conflicted files:"
git diff --name-only --diff-filter=U | sed 's/^/  /'
echo "Launching Zorro..."
cd "$ROOT"
exec cargo run -q -p zorro -- "$DEMO"
