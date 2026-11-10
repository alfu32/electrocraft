# Recorded catalog responses

Tests replay these through `MockTransport`; CI never calls a live service.

* `jlcsearch_10k_0603.json`: recorded from `https://jlcsearch.tscircuit.com/api/search?q=10k%200603&limit=3` on 2026-11-10.
* `digikey_*.json`, `mouser_*.json`, `tme_*.json`, `farnell_*.json`: these APIs need a
  personal key, so the files follow the response schemas documented by each
  provider (field names and nesting as published, values from the public
  product pages). Replace them with real recordings when a key is at hand;
  never commit a key or a response that echoes one.
