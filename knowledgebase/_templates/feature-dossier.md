---
type: feature
domain:
created: {{date:YYYY-MM-DD}}
---

# [Feature Name]

> [One sentence: what this gives the user]

## Overview

[2-3 paragraphs: what the feature does, what problem it solves, who uses it.
Written for a new engineer on their first day. Tone — like explaining to a
colleague over coffee.]

## Architecture

### Data Flow

```text
[ASCII diagram of data flow through system layers]
```

### Components

| Layer | File | What It Does |
|-------|------|-------------|
| Server | `crates/server/src/...` | ... |
| Schema | `crates/schema/src/...` | ... |
| SQLGen | `crates/sqlgen/src/...` | ... |
| Metadata | `crates/metadata/src/...` | ... |

## How It Works

[The most valuable section. Architectural decisions, tricks, non-obvious
things. Write what you'd want to tell a colleague over coffee. This section
is the reason the entire document exists.]

## Configuration

[Env variables, metadata keys, config if any]

## See Also

- [[_index|Knowledge Base Root]]
