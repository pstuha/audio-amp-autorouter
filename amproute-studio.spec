# -*- mode: python ; coding: utf-8 -*-

block_cipher = None

a = Analysis(
    ['amproute_studio\\__main__.py'],
    pathex=[],
    binaries=[],
    datas=[
        ('amproute_studio\\core', 'amproute_studio\\core'),
        ('amproute_studio\\adapters', 'amproute_studio\\adapters'),
        ('amproute_studio\\gui', 'amproute_studio\\gui'),
    ],
    hiddenimports=[
        'PySide6', 'shapely', 'numpy', 'matplotlib',
        'amproute_studio.core.models',
        'amproute_studio.core.routing',
        'amproute_studio.core.placement',
        'amproute_studio.core.simulations',
        'amproute_studio.adapters.kicad_pcb',
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.zipfiles,
    a.datas,
    [],
    name='amproute-studio',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=None,
    version='version_info.txt',
)
