---
name: add-provider
description: Add or update a custom model provider in Ante's catalog.json — an OpenAI-compatible endpoint (Ollama, vLLM, llama.cpp server, an API gateway) or another wire style — discover its models, and declare per-model effort ladders. Use when the user wants to connect a custom or self-hosted endpoint, add models to a custom provider, or fix a missing or wrong effort slider for a custom-provider model. Not for built-in providers: those sign in with /providers or ante auth.
---

# Add a custom provider

Secrets rule: never write an API key into `catalog.json`, never echo one, never
put one as a literal in a command line that gets shown. The entry names its
credential by reference (`env:VAR`, or a key the user stores with `ante auth
add`); check an env var is set with `[ -n "$VAR" ]`.

## 1. Read what exists

The catalog is `$ANTE_HOME/catalog.json`, else `~/.ante/catalog.json`. Read it
first (it may not exist); you merge into it and never drop entries you did not
touch. Confirm with the user:

- provider id (kebab-case, the JSON map key) and display name
- `base_url` — OpenAI-compatible servers usually end in `/v1`
- `wire_style` — `OpenAiCompatible`, `AnthropicMessage`, `OpenAiResponse`, or `Gemini`
- how it authenticates: an env var holding the key (e.g. `MY_PROVIDER_API_KEY`),
  a key the user stores with `ante auth add <id>`, or none (most local servers)

## 2. Discover models

```bash
curl -sf "$BASE_URL/models" -H "Authorization: Bearer $MY_PROVIDER_API_KEY"
```

Drop the header for a server without auth. For a stored key, ask the user for
the model ids instead of handling the key. Ollama also serves `/api/tags` on the
origin with richer metadata. Show the ids and let the user pick — one entry per model they will use, not the listing.

## 3. Write the entry

```json
{
  "providers": {
    "my-provider": {
      "display_name": "My Provider",
      "base_url": "https://api.example.com/v1",
      "credential": "env:MY_PROVIDER_API_KEY",
      "wire_style": "OpenAiCompatible",
      "preferred_models": [
        {
          "id": "some-model",
          "description": "Some Model",
          "context_limit": 262144,
          "effort": "high",
          "supported_efforts": ["low", "medium", "high"]
        }
      ]
    }
  }
}
```

- `credential`: `"env:VAR"` reads the environment; omitted, it is the key stored
  under the provider id, so the user runs `ante auth add <id>` and pastes it;
  `null` sends nothing — required for an unauthenticated local server, or every
  request fails with "No credential for <id>". It is sent as
  `Authorization: Bearer`; add `"credential_header": "x-api-key"` to send it raw
  in that header instead.
- A new provider needs `base_url` and `wire_style`; for an existing id the entry
  is an overlay and models merge by id.
- Optional provider-level `max_request_images` caps images across each request,
  including user images and tool results, for every model on that provider.
  The newest N images are kept in request order; older ones become omission
  markers without changing saved history. Omitted leaves the cap unchanged
  (uncapped by default), `null` clears it, and `0` omits all images. Set a cap
  only when appropriate for the configured endpoint; there is no universal default.
- `effort` is the default slider position. `supported_efforts` is the ladder,
  from `min`, `low`, `medium`, `high`, `xhigh`, `max` (`min` is sent as `minimal`
  on the wire); `[]` means the model takes no effort setting. Omit it to keep
  Ante's built-in ladder — only probe an endpoint (tiny chat request per
  `reasoning_effort` value, ask first on a paid one) when the user wants the
  exact ladder.
- If both are set, `max_tokens` must be below `context_limit` or the model is
  skipped at load.

## 4. Validate

1. `ante catalog` must list the provider on stdout with no
   `warning: skipped catalog entry` on stderr.
2. If the env var is not exported yet, tell the user to add it to their shell
   profile; for a stored key, `ante auth list` shows it with the provider under
   USED BY.
3. The catalog is read once at startup: restart Ante, then `/models` shows the
   models and the slider shows the configured ladder.

If a previously working entry is now skipped after an update, check the
"After an update" section of the `ante-guide` skill.
