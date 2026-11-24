-- Schema of Lutris' pga.db (lutris/database/schema.py, Lutris 0.5.x)
CREATE TABLE games (
    id INTEGER PRIMARY KEY,
    name TEXT,
    sortname TEXT,
    slug TEXT,
    installer_slug TEXT,
    parent_slug TEXT,
    platform TEXT,
    runner TEXT,
    executable TEXT,
    directory TEXT,
    updated DATETIME,
    lastplayed INTEGER,
    installed INTEGER,
    installed_at INTEGER,
    year INTEGER,
    configpath TEXT,
    has_custom_banner INTEGER,
    has_custom_icon INTEGER,
    has_custom_coverart_big INTEGER,
    playtime REAL,
    service TEXT,
    service_id TEXT,
    discord_id TEXT
);
CREATE TABLE sources (id INTEGER PRIMARY KEY, uri TEXT UNIQUE);
CREATE TABLE categories (id INTEGER PRIMARY KEY, name TEXT UNIQUE);
CREATE TABLE games_categories (game_id INTEGER, category_id INTEGER);
CREATE TABLE service_games (id INTEGER PRIMARY KEY, service TEXT, appid TEXT, name TEXT, slug TEXT, icon TEXT, logo TEXT, url TEXT, details TEXT, lutris_slug TEXT);
