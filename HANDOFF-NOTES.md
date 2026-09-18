# Spork — handoff notes (quick pick-up)

## Where things stand

**Repos (renamed on GitHub + locally):**
- `forkit`   → `sporkit`      (protocol / Anchor workspace)  — this repo
- `forkit-site` → `sporkit-site` (Next.js app / web app)
- `forkme`   → deleted (folded into sporkit-site; Capacitor/PWA/QR already live there)

Remotes updated to the new repo names. Watch out: the `bash` tool's persistent cwd
is stale (still points at old paths), so **always** pass an explicit `workdir` and
run one command per call — parallel batch outputs get cross-labeled in this
environment and have burned the most time.

## Program rewrite (the actual current task — IN PROGRESS)

Goal: single slim escrow program, stablecoin + flat SOL fee, mirroring the upgrade
to the Anchor 0.30 toolchain, replacing the old multi-crate forkit stack
(forkit_escrow / forkit_registry / forkit_loyalty / forkit_token) which is staged
for deletion (`git rm`).

Current state of `programs/spork_escrow/`:
- `Cargo.toml` — `anchor-lang = "0.30.1"` + `anchor-spl = "0.30.1"` (use the
  `anchor_spl::token::{token::transfer}` CPI helpers), declare id = program keypair
- `src/lib.rs` — entrypoints only (addressed right before this note)
- `src/state.rs`, `src/errors.rs` — were rewritten mid-session and are now
  **inconsistent with lib.rs** (lib.rs references `state.rs` items by names that
  no longer exist: `Config`, `Order`, `OrderStatus`, `SporkError` variants).
  This is the #1 thing to fix — rewrite all three files as one coherent set.

Program ID: use `solana -k spork-escrow-keypair.json address` to derive the real
program ID (cropped/ambiguous in shell history) — verify it matches `declare_id!`
before building. Keep `spork-escrow-keypair.json` **gitignored** (it is a secret —
DO NOT commit; the .gitignore currently has no entry for it).

Design locked earlier (approved by user, don't re-litigate):
- Stablecoin escrow (not SOL escrow), vault = token account PDA
- Flat protocol fee → fee_wallet, with 2-step timelocked fee-wallet change
- Lifecycle: Created → Accepted → Preparing → ReadyForPickup → PickedUp →
  Delivered → Settled (+ Cancelled / Refunded / Disputed, 2-of-3 arbiter
  dispute resolution)
- Seeds: `["spork","config"]`, `["spork","order",order_id]`,
  `["spork","vault",order_id]` + vault authority PDA
- No token/loyalty/registry programs; single `spork_escrow` crate in workspace

## Next actions (in order)
1. Rewrite `state.rs` (Config, Order, OrderStatus), `errors.rs` (SporkError),
   `lib.rs` (accounts + handlers) as **one coherent set** — cross-check every
   name between the 3 files.
2. `anchor build` — fix until clean.
3. `anchor test` (write `tests/spork_escrow.ts` when the program compiles).
4. Clean stale dirs / `.env` / `Anchor.toml` IDs to the single program.
5. Later: Capacitor/PWA already in site; rebrand ForkIt→Spork throughout.

## Environment quirks (read before running anything)
- Home is `/home/douglas`; git config says user `douglasdemaio` / `douglas`.
- `/tmp/opencode` exists for scratch. Use explicit `workdir` on every bash call.
- Anchor toolchain 0.30.1; anchor-spl 0.30.1 (use `anchor_spl::token::token::...`).
