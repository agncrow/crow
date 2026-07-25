CREATE TABLE trades (
  id     INTEGER PRIMARY KEY,
  symbol TEXT NOT NULL,
  qty    INTEGER NOT NULL,
  entry  REAL NOT NULL,
  status TEXT NOT NULL DEFAULT 'active'
);
