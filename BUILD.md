# AmpRoute Studio Build Instructions

## Prerequisites
- Python 3.11+
- pip

## Install development dependencies
```bash
pip install -e ".[dev,gui,sim]"
```

## Build exe with PyInstaller

### Full build (includes GUI)
```bash
pyinstaller amproute-studio.spec --clean
```

Output will be in `dist/` folder:
- `amproute-studio.exe` - GUI application

### Or use the build script
```bash
python build.py
```

## Direct run (no exe needed)
```bash
pip install -e ".[all]"
amprote --help        # CLI
amprote-studio         # GUI
```

## Files
- `amproute-studio.spec` - PyInstaller spec file
- `build.py` - Build automation script
