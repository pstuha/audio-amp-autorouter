"""Build script for AmpRoute Studio exe."""

import subprocess
import sys
import os
from pathlib import Path

def build():
    """Build exe with PyInstaller."""
    spec_file = Path(__file__).parent / "amproute-studio.spec"
    
    if not spec_file.exists():
        print("ERROR: amproute-studio.spec not found")
        sys.exit(1)
    
    # Install pyinstaller if needed
    try:
        import PyInstaller
    except ImportError:
        print("Installing PyInstaller...")
        subprocess.check_call([sys.executable, "-m", "pip", "install", "pyinstaller"])
    
    # Clean old builds
    dist_dir = Path(__file__).parent / "dist"
    if dist_dir.exists():
        print("Cleaning old dist folder...")
        import shutil
        shutil.rmtree(dist_dir)
    
    # Build
    print("Building exe with PyInstaller...")
    result = subprocess.run(
        [sys.executable, "-m", "PyInstaller", str(spec_file), "--clean"],
        capture_output=False
    )
    
    if result == 0:
        exe_path = dist_dir / "amproute-studio.exe"
        if exe_path.exists():
            print(f"SUCCESS: exe created at {exe_path}")
        else:
            print("WARNING: build completed but exe not found")
    else:
        print("ERROR: build failed")
        sys.exit(1)

if __name__ == "__main__":
    build()
