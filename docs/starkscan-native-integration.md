# uc and Starkscan

How `uc` uses Starkscan (repository `starknet-innovation/mezcal`) and what Starkscan can take
from `uc`. Everything here is implemented and tested against the live API on 2026-10-25.

## Starkscan's API, as uc uses it

Base URL `https://starkscan.co/api`, routes under `/v1/{chain}/` with `chain` = `SN_MAIN` or
`SN_SEPOLIA`, authenticated with the `X-Starkscan-Api-Key` header. `uc` reads the key from
`STARKSCAN_API_KEY`. Without a key, requests to the gateway answer
`{"code": "authentication_required", "httpStatus": 401}`, and `uc` falls back to the public
Cartridge RPC for `--network` presets.

| Route | Used by |
| :--- | :--- |
| `POST /v1/{chain}/rpc` (Starknet JSON-RPC) | every on-chain command once a key is set; `--url https://starkscan.co/rpc/v0_10/{chain}/<key>` is the URL-token form |
| `GET /v1/{chain}/contract/{address}` (`alias`, `isAccount`, `isToken`, `tokenKind`, `classHash`) | `uc trace` address labels |
| `GET /v1/{chain}/token/{address}` (`symbol`, `name`, `decimals`) | `uc trace` token labels |
| `GET /v1/{chain}/contract/{address}/verification` | `uc audit <address>`, `uc verify-onchain` (`starkscan_verification_record`) |
| `PUT /v1/{chain}/contract/{address}/verification` | `uc verify-onchain --submit` |

The verification record is camelCase: `classHash`, `sourceRef`, `language`, `compilerVersion`,
`verificationStatus` (`pending`, `verified`, `failed`), `verificationError`, `requestedAtIso`,
`verifiedAtIso`, `metadata` (object, size-bounded). `uc verify-onchain` builds the upsert body
from its local recompilation: status `verified` when the local class hash equals the deployed
one, `failed` with an error otherwise, `sourceRef` the exported bundle path or
`uc-bundle:<source tree merkle root>`, and `metadata` carrying the bundle schema, merkle root,
file count and uc version. The PUT needs a key with the verification scope.

## What Starkscan can take from uc

- **Audit feed.** `uc audit --declared-since <block> --declared-until <block> --json` walks
  `starknet_getStateUpdate` for the range, fetches every Sierra class in `declared_classes`,
  runs the symbolic prover and the Sierra detectors, and emits one entry per class:
  `block_number`, `class_hash`, `grade` (A+, A, B, C), `functions`, `proven_safe`,
  `guarded_safe`, `counterexamples[]`, `findings[]` (`severity detector function`),
  `elapsed_ms`. Cairo 0 declarations appear with `audited: false`. `--max-classes` bounds a
  run; the report carries `from_block`, `to_block` and `blocks_scanned` so a cron job can
  resume from `to_block + 1`.
- **Per-address watch.** `uc watch <address> --once --json` re-audits when the class hash at
  the address changes and lists new and resolved findings against the previous class.
- **Verification.** The deterministic bundle (`uc-starkscan-verification-bundle-v1`: sources,
  manifest, compiler versions, expected class hash, source tree merkle root) plus the in-memory
  verifier (`uc verify-onchain --verify-bundle <file>`) recompile sources and check class hash
  parity against the chain, byte-identical to scarb 2.18 artifacts.

## Not done

- Starkscan has no storage-slot name endpoint; `uc prove --check-upgrade 0x...` reports removed
  slots of an unnamed deployed class by address.
- The Starkscan verification PUT is not exercised in CI (it needs a scoped key); the GET and
  the RPC gateway are.
