INSERT INTO games (id, name, sortname, slug, installer_slug, platform, runner, executable, directory, lastplayed, installed, installed_at, year, configpath, playtime, service, service_id)
VALUES (1, 'The Witcher 3: Wild Hunt', '', 'the-witcher-3', 'the-witcher-3-gog', 'Windows', 'wine', NULL, '/home/u/Games/the-witcher-3', 1700000000, 1, 1690000000, 2015, 'the-witcher-3-1690000000', 12.5, 'gog', '1207664663');
INSERT INTO games (id, name, slug, platform, runner, directory, lastplayed, installed, installed_at, year, configpath, playtime)
VALUES (2, 'SuperTuxKart', 'supertuxkart', 'Linux', 'linux', '/home/u/Games/supertuxkart', 0, 1, 1690000001, 2007, 'supertuxkart-1690000001', 0.0);
INSERT INTO games (id, name, slug, platform, runner, directory, lastplayed, installed, year, configpath, playtime)
VALUES (3, 'Not Installed Yet', 'not-installed-yet', '', '', '', NULL, 0, NULL, '', NULL);
INSERT INTO games (id, name, slug, installer_slug, platform, runner, directory, lastplayed, installed, installed_at, year, configpath, playtime)
VALUES (4, 'Battle.net', 'battlenet', 'battlenet-standard', 'Windows', 'wine', '/home/u/Games/battlenet', 1795299584, 1, 1795299275, 1996, 'battlenet-standard-1795299275', 0.08),
       (5, 'EA App', 'ea-app', 'ea-app-standard', 'Windows', 'wine', '/home/u/Games/ea-app', 1795300206, 1, 1795299452, 2022, 'ea-app-standard-1795299452', 0.2),
       (6, 'Ubisoft Connect', 'ubisoft-connect', 'ubisoft-connect-latest', 'Windows', 'wine', '/home/u/Games/ubisoft-connect', 1795300524, 1, 1795300122, 2020, 'ubisoft-connect-latest-1795300122', 0.09),
       (7, 'Prism Launcher', 'prism-launcher', 'prism-launcher-appimage', 'Linux', 'linux', '/home/u/Games/prism-launcher', 1795300602, 1, 1795300562, 2022, 'prism-launcher-appimage-1795300561', 0.01);
INSERT INTO categories (id, name) VALUES (1, '.hidden'), (2, 'favorite');
INSERT INTO games_categories (game_id, category_id) VALUES (2, 1);
