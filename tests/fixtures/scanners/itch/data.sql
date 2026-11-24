INSERT INTO games (id, url, title, short_text, type, classification, cover_url, still_cover_url)
VALUES (90058, 'https://mattmakesgames.itch.io/celeste', 'Celeste', 'Help Madeline survive her inner demons', 'default', 'game', 'https://img.itch.zone/aW1nLzEwMjU4NjYucG5n/315x250%23c/abcd.png', NULL),
       (12345, 'https://someone.itch.io/windows-thing', 'Windows Thing', '', 'default', 'game', NULL, NULL),
       (999, 'https://someone.itch.io/not-installed', 'Not Installed', '', 'default', 'game', NULL, NULL);
INSERT INTO install_locations (id, path) VALUES ('appdata', '/HOME/.config/itch/apps'), ('games', '/HOME/Games/itch');
INSERT INTO caves (id, game_id, upload_id, installed_at, last_touched_at, seconds_run, verdict, installed_size, install_location_id, install_folder_name, custom)
VALUES ('0b5ac1e5-6f2d-4a3e-9d4c-1a2b3c4d5e6f', 90058, 1, '2023-04-01T10:00:00.123456789Z', '2023-05-01T20:00:00.000000001Z', 7200,
        '{"basePath":"/HOME/.config/itch/apps/celeste","totalSize":1234,"candidates":[{"path":"Celeste","depth":1,"flavor":"linux","arch":"amd64","size":1234}]}',
        1234, 'appdata', 'celeste', 0),
       ('a1b2c3d4-0000-4000-8000-00000000beef', 12345, 2, '2023-04-01T10:00:00Z', NULL, 0,
        '{"totalSize":10,"candidates":[{"path":"thing.exe","depth":1,"flavor":"windows","arch":"386","size":10}]}',
        10, 'games', 'windows-thing', 0);
