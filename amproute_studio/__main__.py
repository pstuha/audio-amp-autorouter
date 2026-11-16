"""AmpRoute Studio entry point."""

import sys
import os

# For PyInstaller frozen executable
if getattr(sys, 'frozen', False):
    # Running as compiled exe
    if hasattr(sys, '_MEIPASS'):
        sys.path.insert(0, sys._MEIPASS)
    # Try to import and run GUI
    try:
        from amproute_studio.gui.app import run
        raise SystemExit(run())
    except Exception as e:
        # If GUI fails, at least show CLI
        os.system('amprote --help')
        print(f"GUI error: {e}")
        raise SystemExit(1)
else:
    # Running as Python module
    try:
        from amproute_studio.gui.app import run
    except ImportError:
        # Fallback for development
        from .gui.app import run
    raise SystemExit(run())
