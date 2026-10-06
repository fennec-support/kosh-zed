# koshka for Zed

This repository hosts [Koshka Shell](https://github.com/fennec-support/kosh)
tooling for Zed.

Currently that includes:
- Linter
- LSP/Symbols
- Formatter

You should probably install this extension from the Zed marketplace.

### Manual install

Install the WebAssembly target:

```bash
rustup target add wasm32-wasip2
```

Then open the command palette, run `zed: install dev extension`, and select
this directory. Zed builds and reloads the extension.

### Settings

```jsonc
{
  "file_types": {
    "Shell Script": ["kosh", "shit"]
  },
  "languages": {
    "Shell Script": {
      "language_servers": ["kosh", "..."],
      "formatter": { "language_server": { "name": "kosh" } },
      "format_on_save": "off"
    },
    "YAML": { "language_servers": ["kosh", "..."] },
    "Markdown": { "language_servers": ["kosh", "..."] },
    "Dockerfile": { "language_servers": ["kosh", "..."] },
    "Docker Compose": { "language_servers": ["kosh", "..."] },
    "Make": { "language_servers": ["kosh", "..."] },
    "JSON": { "language_servers": ["kosh", "..."] },
    "JSONC": { "language_servers": ["kosh", "..."] }
  }
}
```

Finding the shell:
```jsonc
{
  "lsp": {
    "kosh": {
      "binary": {
        "path": "/usr/local/bin/kosh"
      }
    }
  }
}
```

The extension appends `--as-language-server` if the `binary.arguments` omits it.

`kosh --format` can format a file without the language server.

```jsonc
{
  "languages": {
    "Shell Script": {
      "formatter": {
        "external": {
          "command": "kosh",
          "arguments": ["--format"]
        }
      }
    }
  }
}
```

Zed writes the buffer to standard input without a filename and `kosh` treats
every buffer as a plain shell script. Configure this for shell languages only.
