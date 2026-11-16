# One scenario per transition, one file per lot (shell.mjs, menus.mjs, …).

Each file exports `scenarios`: `{ name, setup(page), act(page), settle(page) }`.
