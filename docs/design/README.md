# Design

How Messaging MCP is built, in reading order. Each note describes the code as it is today.

| Note | Covers |
|---|---|
| [01-architecture.md](01-architecture.md) | The parts, the life of a plan, the MCP tools and the CLI commands |
| [02-setup.md](02-setup.md) | Installing and registering, the setup the assistant runs, how messages are approved, every config field, secrets |
| [03-telegram.md](03-telegram.md) | Creating the app, the QR login, how a send works, finding and allowing people |
| [04-whatsapp.md](04-whatsapp.md) | Our WhatsApp bridge: its routes, how the assistant starts and pairs it, allowing people |
| [05-storage.md](05-storage.md) | The config folder and the ledger: tables, columns, migrations and file permissions |
| [06-testing.md](06-testing.md) | The offline suite, the coverage floor, and the live tests |
