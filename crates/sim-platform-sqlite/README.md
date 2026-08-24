# sim-platform-sqlite

The physical SQLite capsule for SIM's provider-neutral relation site. It owns
`rusqlite`, private checked-plan lowering, prepared statement values, preopened
path resolution, catalog introspection, atomic migration attestations, safe
attach lifecycle, bounded interruption, and stable error mapping.

Consumers load `relation/site:sqlite` and pass only closed logical locators and
checked relation plans. Native paths, SQL text, and driver values never cross
that contract.
