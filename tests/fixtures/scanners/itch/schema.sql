-- Subset of butler's database schema (butler/database/models), the tables/columns the scanner reads plus neighbours.
CREATE TABLE games (
    id INTEGER PRIMARY KEY,
    url TEXT,
    title TEXT,
    short_text TEXT,
    type TEXT,
    classification TEXT,
    cover_url TEXT,
    still_cover_url TEXT,
    created_at DATETIME,
    published_at DATETIME,
    min_price INTEGER,
    can_be_bought BOOLEAN,
    has_demo BOOLEAN,
    in_press_system BOOLEAN,
    user_id INTEGER
);
CREATE TABLE install_locations (
    id TEXT PRIMARY KEY,
    path TEXT
);
CREATE TABLE caves (
    id TEXT PRIMARY KEY,
    game_id INTEGER,
    external_game_id INTEGER,
    upload_id INTEGER,
    build_id INTEGER,
    upload TEXT,
    build TEXT,
    installed_at DATETIME,
    last_touched_at DATETIME,
    seconds_run INTEGER,
    verdict TEXT,
    installed_size INTEGER,
    install_location_id TEXT,
    install_folder_name TEXT,
    custom BOOLEAN
);
