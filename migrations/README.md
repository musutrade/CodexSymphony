# Database migrations

The server runs SQLx migrations at startup before accepting HTTP requests.
The initial skeleton contains no business tables yet. Add versioned SQL files
here with the first domain persistence task; never modify an applied migration.
Integration tests require a dedicated TEST_DATABASE_URL and never infer a production URL.
