# Database Migrations

## Log Format

Add one entry per schema change:

1. **Version**: short stable id (for example `2026-04-14-sort-order`).
2. **Change**: exact SQL operation and target table/column.
3. **Guard**: how startup code makes it idempotent.
4. **Verification**: command or test proving migration succeeded.

Use this template:

```md
## <version>
- Change: <SQL or schema change>
- Guard: <idempotency check>
- Verification: <test/command>
```

## Migration Log

## 2026-04-14-sort-order
- Change: `ALTER TABLE command_nodes ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0`
- Guard: `init_db` inspects `PRAGMA table_info(command_nodes)` and only alters when column missing.
- Verification: `cargo test db::reorder::sort_order_migration_adds_missing_column`
