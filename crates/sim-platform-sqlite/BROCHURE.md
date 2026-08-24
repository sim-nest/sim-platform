# Attesting SQLite relations

Run the same checked relational plans against memory or durable preopened
SQLite files without making SQL or native paths part of the application.
Schema state is re-introspected and content-attested, migrations are atomic,
attached sources are named and bounded, and provider failures become stable
relation errors.
