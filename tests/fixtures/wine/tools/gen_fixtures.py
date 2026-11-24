#!/usr/bin/env python3
"""Generate tests/fixtures/wine/home (run from anywhere; arg = repo root)."""
import os, sys, struct, shutil, json
root = os.path.join(sys.argv[1], "tests/fixtures/wine/home")
if os.path.exists(root):
    shutil.rmtree(root)
H = "@HOME@"

def w(rel, data, mode=None):
    p = os.path.join(root, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "wb") as f:
        f.write(data.encode() if isinstance(data, str) else data)
    if mode:
        os.chmod(p, mode)

def stub(rel):  # placeholder program / marker file
    w(rel, "MZ fixture stub\n")

# ---- protobuf helpers
def varint(n):
    out = b""
    while True:
        b = n & 0x7f; n >>= 7
        if n: out += bytes([b | 0x80])
        else: return out + bytes([b])
def key(f, t): return varint(f << 3 | t)
def ln(f, b): b = b.encode() if isinstance(b, str) else b; return key(f, 2) + varint(len(b)) + b
def vi(f, n): return key(f, 0) + varint(n)
def install(uid, code, path, installed=True, extra=b""):
    settings = ln(1, path) + ln(2, "eu") + ln(6, "enUS")
    base = vi(1, 1 if installed else 0) + vi(2, 1 if installed else 0) + ln(7, "1.2.3.45678")
    state = ln(1, base)
    return ln(1, ln(1, uid) + ln(2, code) + ln(3, settings) + ln(4, state) + extra)

REG_HDR = "WINE REGISTRY Version 2\n;; All keys relative to \\\\Machine\n\n#arch=win64\n\n"
USER_HDR = "WINE REGISTRY Version 2\n;; All keys relative to \\\\User\\\\S-1-5-21-0-0-0-1000\n\n#arch=win64\n\n"
def prefix(rel, system_extra="", user_extra=""):
    w(rel + "/system.reg", REG_HDR + "[Software\\\\Wine] 1700000000\n#time=1da0000000000000\n\"Version\"=\"win10\"\n\n" + system_extra)
    w(rel + "/user.reg", USER_HDR + user_extra)
    w(rel + "/userdef.reg", USER_HDR)
    stub(rel + "/drive_c/windows/system32/kernel32.dll")

# ---- Steam root + Proton build
st = ".local/share/Steam"
w(st + "/compatibilitytools.d/Fixture-Proton/proton", "#!/bin/sh\nexit 0\n", 0o755)
w(st + "/compatibilitytools.d/Fixture-Proton/version", "1700000000 fixture-1.0\n")
w(st + "/compatibilitytools.d/Fixture-Proton/compatibilitytool.vdf",
  '"compatibilitytools"\n{\n  "compat_tools"\n  {\n    "fixture_proton"\n    {\n      "install_path" "."\n      "display_name" "Fixture Proton"\n      "from_oslist" "windows"\n      "to_oslist" "linux"\n    }\n  }\n}\n')
w(st + "/config/config.vdf",
  '"InstallConfigStore"\n{\n\t"Software"\n\t{\n\t\t"Valve"\n\t\t{\n\t\t\t"Steam"\n\t\t\t{\n\t\t\t\t"CompatToolMapping"\n\t\t\t\t{\n\t\t\t\t\t"3000000001"\n\t\t\t\t\t{\n\t\t\t\t\t\t"name"\t\t"fixture_proton"\n\t\t\t\t\t\t"config"\t\t""\n\t\t\t\t\t\t"priority"\t\t"250"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n')
w(st + "/steamapps/libraryfolders.vdf", '"libraryfolders"\n{\n}\n')
# shortcuts.vdf (binary KeyValues)
def bstr(k, v): return b"\x01" + k.encode() + b"\0" + v.encode() + b"\0"
def bint(k, v): return b"\x02" + k.encode() + b"\0" + struct.pack("<i", v)
appid = 3000000001
sc = b"\x00shortcuts\x00" + b"\x000\x00" + bint("appid", appid - (1 << 32)) + bstr("AppName", "Battle.net") \
    + bstr("Exe", '"/home/u/bnet/Battle.net Launcher.exe"') + bstr("StartDir", '"/home/u/bnet/"') + b"\x08" + b"\x08\x08"
w(st + "/userdata/12345/config/shortcuts.vdf", sc)
# Steam shortcut prefix: a second Battle.net with Overwatch (same game as in Faugus)
cd = st + "/steamapps/compatdata/3000000001"
w(cd + "/version", "fixture-1.0\n")
prefix(cd + "/pfx")
stub(cd + "/pfx/drive_c/Program Files (x86)/Battle.net/Battle.net.exe")
stub(cd + "/pfx/drive_c/Program Files (x86)/Overwatch/Overwatch Launcher.exe")
w(cd + "/pfx/drive_c/ProgramData/Battle.net/Agent/product.db",
  install("battle.net", "bna", "C:/Program Files (x86)/Battle.net") + install("prometheus", "Pro", "C:/Program Files (x86)/Overwatch"))
# A real Steam game's compatdata: not a shortcut, ignored.
prefix(st + "/steamapps/compatdata/730/pfx")

# ---- Faugus: Battle.net prefix (games.json) + default-prefix folder
w(".local/share/faugus-launcher/games.json", json.dumps([
    {"gameid": "battlenet", "title": "Battle.net", "path": H + "/Faugus/battlenet/drive_c/Program Files (x86)/Battle.net/Battle.net Launcher.exe",
     "prefix": H + "/Faugus/battlenet", "runner": "Fixture-Proton", "playtime": 0},
], indent=4))
w(".config/faugus-launcher/config.json", json.dumps({"default-prefix": H + "/Faugus", "default-runner": "Fixture-Proton"}, indent=4))
fb = "Faugus/battlenet"
prefix(fb)
w(fb + "/version", "fixture-1.0\n")
stub(fb + "/drive_c/Program Files (x86)/Battle.net/Battle.net.exe")
stub(fb + "/drive_c/Program Files (x86)/Battle.net/Battle.net Launcher.exe")
stub(fb + "/drive_c/Program Files (x86)/Overwatch/Overwatch Launcher.exe")
stub(fb + "/drive_c/Program Files (x86)/Diablo IV/Diablo IV Launcher.exe")
stub(fb + "/drive_c/Program Files (x86)/Mystery Game/game.exe")
stub(fb + "/drive_c/Program Files (x86)/StarCraft II/StarCraft II.exe")
db = (install("agent", "agent", "C:/ProgramData/Battle.net/Agent")
      + install("battle.net", "bna", "C:/Program Files (x86)/Battle.net")
      + install("prometheus", "pro", "C:/Program Files (x86)/Overwatch", extra=ln(5, ln(1, b"\x08\x01")))
      + install("fenris", "Fen", "C:\\Program Files (x86)\\Diablo IV")
      + install("hs_beta", "WTCG", "C:/Program Files (x86)/Hearthstone")      # folder gone: skipped
      + install("s2", "S2", "C:/Program Files (x86)/StarCraft II", installed=False)  # still installing: skipped
      + install("zzz", "ZZZ", "C:/Program Files (x86)/Mystery Game")
      + ln(2, ln(1, "prometheus") + ln(2, "Pro"))   # active_installs: ignored
      + vi(6, 42) + ln(7, "prometheus"))
w(fb + "/drive_c/ProgramData/Battle.net/Agent/product.db", db)
prefix("Faugus/empty")

# ---- Lutris: Ubisoft + EA + Rockstar in ~/Games/ubisoft (wine runner)
w(".config/lutris/games/ubisoft-connect-1700000000.yml",
  "game:\n  exe: " + H + "/Games/ubisoft/drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/UbisoftConnect.exe\n"
  "  prefix: " + H + "/Games/ubisoft\n  arch: win64\nsystem:\n  env:\n    DXVK_HUD: '0'\nwine:\n  version: wine-ge-8-26-x86_64\n  dxvk: true\n")
w(".config/lutris/games/notwine-1700000001.yml", "game:\n  exe: /usr/bin/true\nsystem: {}\n")
w(".local/share/lutris/runners/wine/wine-ge-8-26-x86_64/bin/wine", "#!/bin/sh\nexit 0\n", 0o755)
gu = "Games/ubisoft"
ubi = "C:\\\\Program Files (x86)\\\\Ubisoft\\\\Ubisoft Game Launcher"
system = f"""[Software\\\\Wow6432Node\\\\Ubisoft\\\\Launcher] 1700000000
#time=1da0000000000000
"InstallDir"="{ubi}\\\\"
"Version"=dword:00000096

[Software\\\\Wow6432Node\\\\Ubisoft\\\\Launcher\\\\Installs\\\\635] 1700000000
#time=1da0000000000000
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Assassin's Creed Origins/"
"Language"="en-US"

[Software\\\\Wow6432Node\\\\Ubisoft\\\\Launcher\\\\Installs\\\\5595] 1700000000
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Far Cry 6/"

[Software\\\\Wow6432Node\\\\Ubisoft\\\\Launcher\\\\Installs\\\\999] 1700000000
"InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Removed Game/"

[Software\\\\Wow6432Node\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\Uplay Install 635] 1700000000
"DisplayName"="Assassin's Creed Origins"
"InstallLocation"="C:\\\\Program Files (x86)\\\\Ubisoft\\\\Ubisoft Game Launcher\\\\games\\\\Assassin's Creed Origins\\\\"

[Software\\\\Electronic Arts\\\\EA Desktop] 1700000000
"ClientPath"="C:\\\\Program Files\\\\Electronic Arts\\\\EA Desktop\\\\EA Desktop\\\\EADesktop.exe"
"InstallLocation"="C:\\\\Program Files\\\\Electronic Arts\\\\EA Desktop\\\\EA Desktop"
"InstallSuccessful"="true"

[Software\\\\Wow6432Node\\\\Electronic Arts\\\\EA Core] 1700000000
"EADM6InstallDir"="C:\\\\Program Files\\\\Electronic Arts\\\\EA Desktop\\\\EA Desktop"

[Software\\\\Wow6432Node\\\\EA Games\\\\Mass Effect Legendary Edition] 1700000000
"Install Dir"="C:\\\\Program Files\\\\EA Games\\\\Mass Effect Legendary Edition\\\\"
"Locale"="en_US"
"DisplayName"="Mass Effect Legendary Edition"

[Software\\\\Wow6432Node\\\\EA Games\\\\Battlefield 1] 1700000000
"Install Dir"="C:\\\\Program Files\\\\EA Games\\\\Battlefield 1\\\\"

[Software\\\\Wow6432Node\\\\Rockstar Games\\\\Launcher] 1700000000
"InstallFolder"="C:\\\\Program Files\\\\Rockstar Games\\\\Launcher"
"Language"="en-US"

[Software\\\\Wow6432Node\\\\Rockstar Games\\\\Grand Theft Auto V] 1700000000
"InstallFolder"="C:\\\\Program Files\\\\Rockstar Games\\\\Grand Theft Auto V"

[Software\\\\Wow6432Node\\\\Rockstar Games\\\\Red Dead Redemption 2] 1700000000
"InstallFolder"="C:\\\\Program Files\\\\Rockstar Games\\\\Red Dead Redemption 2"

[Software\\\\Wow6432Node\\\\Rockstar Games\\\\Rockstar Games Social Club] 1700000000
"InstallFolder"="C:\\\\Program Files\\\\Rockstar Games\\\\Social Club"

[Software\\\\Wow6432Node\\\\Rockstar Games\\\\Max Payne 3] 1700000000
"InstallFolder"="C:\\\\Program Files\\\\Rockstar Games\\\\Max Payne 3"

[Software\\\\Microsoft\\\\Windows NT\\\\CurrentVersion] 1700000000
"ProductName"="Microsoft Windows 10"
"PathExpand"=hex(2):25,00,53,00,79,00,73,00,74,00,65,00,6d,00,52,00,6f,00,6f,00,\\
  74,00,25,00,00,00
"Blob"=hex:de,ad,\\
  be,ef
"Multi"=str(7):"one\\0two\\0"
"Expand"=str(2):"%SystemRoot%\\\\system32"
@="default"
"Uni"="caf\\xe9 \\x4e2d"
"Quote"="say \\"hi\\""

"""
prefix(gu, system_extra=system)
U = gu + "/drive_c/Program Files (x86)/Ubisoft/Ubisoft Game Launcher"
stub(U + "/UbisoftConnect.exe"); stub(U + "/upc.exe")
stub(U + "/games/Assassin's Creed Origins/ACOrigins.exe")
stub(U + "/games/Far Cry 6/bin/FarCry6.exe")
E = gu + "/drive_c/Program Files/Electronic Arts/EA Desktop/EA Desktop"
stub(E + "/EADesktop.exe")
me = gu + "/drive_c/Program Files/EA Games/Mass Effect Legendary Edition"
stub(me + "/Game/Launcher/MassEffectLauncher.exe")
w(me + "/__Installer/installerdata.xml", """<?xml version="1.0" encoding="utf-8"?>
<DiPManifest version="4.0">
  <buildMetaData><featureFlags autoUpdateEnabled="1" /></buildMetaData>
  <contentIDs>
    <contentID>196749</contentID>
    <contentID>1026023</contentID>
  </contentIDs>
  <gameTitles>
    <gameTitle locale="de_DE">Mass Effect™ Legendary Edition</gameTitle>
    <gameTitle locale="en_US">Mass Effect™ Legendary Edition</gameTitle>
  </gameTitles>
  <runtime><launcher><filePath>[HKEY_LOCAL_MACHINE\\SOFTWARE\\BioWare\\Mass Effect Legendary Edition\\Install Dir]Game\\Launcher\\MassEffectLauncher.exe</filePath></launcher></runtime>
</DiPManifest>
""")
stub(gu + "/drive_c/Program Files/EA Games/Battlefield 1/bf1.exe")
w(gu + "/drive_c/ProgramData/Origin/LocalContent/Battlefield 1/Origin.OFR.50.0000557.mfst",
  "?currentstate=kReadyToStart&dipinstallpath=C%3a%5cProgram%20Files%5cEA%20Games%5cBattlefield%201%5c&id=Origin.OFR.50.0000557&previousstate=kInstalling")
R = gu + "/drive_c/Program Files/Rockstar Games"
stub(R + "/Launcher/Launcher.exe")
stub(R + "/Grand Theft Auto V/PlayGTAV.exe")
stub(R + "/Max Payne 3/MaxPayne3.exe")
stub(R + "/Social Club/socialclub.dll")

# ---- Bottles: Riot (soda runner)
bo = ".local/share/bottles"
w(bo + "/runners/soda-9.0-1/bin/wine", "#!/bin/sh\nexit 0\n", 0o755)
w(bo + "/bottles/Riot/bottle.yml", "Arch: win64\nCustom_Path: false\nDXVK: dxvk-2.3\nEnvironment: Gaming\nName: Riot Games\nPath: Riot\nRunner: soda-9.0-1\nVersioning: false\nWindows: win10\nworkdir: ''\n")
rb = bo + "/bottles/Riot"
prefix(rb)
w(rb + "/drive_c/ProgramData/Riot Games/RiotClientInstalls.json", json.dumps({
    "associated_client": {
        "C:/Riot Games/League of Legends/": "C:/Riot Games/Riot Client/RiotClientServices.exe",
        "C:/Riot Games/LoR/": "C:/Riot Games/Riot Client/RiotClientServices.exe"},
    "patchlines": {"KeystoneFoundationLiveWin": "C:/Riot Games/Riot Client/RiotClientServices.exe"},
    "rc_default": "C:/Riot Games/Riot Client/RiotClientServices.exe",
    "rc_live": "C:/Riot Games/Riot Client/RiotClientServices.exe"}, indent=4))
md = rb + "/drive_c/ProgramData/Riot Games/Metadata"
w(md + "/league_of_legends.live/league_of_legends.live.product_settings.yaml",
  'auto_patching_enabled_by_player: true\ndependencies:\n    Direct X 9:\n        hash: ""\n        phase: "Succeeded"\nlocale_data:\n    available_locales:\n    - "en_US"\n    default_locale: "en_US"\npatching_policy: "manual"\npatchline_patching_ask_policy: "ask"\nproduct_install_full_path: "C:/Riot Games/League of Legends"\nproduct_install_root: "C:/Riot Games"\nshortcut_name: "League of Legends.lnk"\nshould_repair: false\n')
w(md + "/valorant.live/valorant.live.product_settings.yaml",
  'product_install_full_path: "C:/Riot Games/VALORANT/live"\nproduct_install_root: "C:/Riot Games"\n')
w(md + "/riot_client/riot_client.product_settings.yaml", 'product_install_full_path: "C:/Riot Games/Riot Client"\n')
stub(rb + "/drive_c/Riot Games/Riot Client/RiotClientServices.exe")
stub(rb + "/drive_c/Riot Games/League of Legends/LeagueClient.exe")
stub(rb + "/drive_c/Riot Games/VALORANT/live/VALORANT.exe")
stub(rb + "/drive_c/Riot Games/LoR/LoR.exe")

# ---- Heroic: Proton-style prefix (pfx/) with HoYoPlay
w(".config/heroic/GamesConfig/Fortnite.json", json.dumps({"Fortnite": {
    "winePrefix": H + "/Games/Heroic/Prefixes/default/HoYo",
    "wineVersion": {"bin": H + "/.local/share/Steam/compatibilitytools.d/Fixture-Proton/proton", "name": "Fixture Proton", "type": "proton"},
    "launcherArgs": ""}}, indent=2))
w(".config/heroic/config.json", json.dumps({"defaultSettings": {"winePrefix": H + "/Games/Heroic/Prefixes/default", "wineVersion": {"bin": "/usr/bin/wine", "name": "Wine Default", "type": "wine"}}}))
hp = "Games/Heroic/Prefixes/default/HoYo"
w(hp + "/version", "fixture-1.0\n")
user = """[Software\\\\Cognosphere\\\\HYP\\\\1_0\\\\hk4e_global] 1700000000
#time=1da0000000000000
"GameInstallPath"="C:\\\\Program Files\\\\HoYoPlay\\\\games\\\\Genshin Impact game"

[Software\\\\Cognosphere\\\\HYP\\\\1_0\\\\nap_global] 1700000000
"GameInstallPath"="C:\\\\Program Files\\\\HoYoPlay\\\\games\\\\ZenlessZoneZero Game"

[Software\\\\Cognosphere\\\\HYP\\\\1_0\\\\bh3_global] 1700000000
"GameInstallPath"="C:\\\\Games\\\\Missing"

"""
prefix(hp + "/pfx", user_extra=user)
HY = hp + "/pfx/drive_c/Program Files/HoYoPlay"
stub(HY + "/launcher.exe")
stub(HY + "/games/Genshin Impact game/GenshinImpact.exe")
stub(HY + "/games/ZenlessZoneZero Game/data.bin")     # exe missing: launched via HoYoPlay
stub(HY + "/games/Star Rail Games/StarRail.exe")      # not in the registry: found by folder
os.makedirs(os.path.join(root, hp + "/pfx/drive_c/users/steamuser/AppData/Roaming/Cognosphere/HYP/1_0"), exist_ok=True)
stub(hp + "/pfx/drive_c/users/steamuser/AppData/Roaming/Cognosphere/HYP/1_0/config.ini")

# ---- umu, ~/Games/*, ~/.wine, extra dirs, decoys
prefix("Games/umu/umu-default"); w("Games/umu/umu-default/version", "UMU-Proton-9.0\n")
prefix("Games/plain")
stub("Games/notaprefix/drive_c/readme.txt")
prefix(".wine")
prefix("extra/one"); prefix("extra/two")
prefix("single")
print("ok", root)
