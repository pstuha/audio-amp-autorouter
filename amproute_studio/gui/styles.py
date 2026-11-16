"""Professional dark theme styles for AmpRoute Studio."""

DARK_STYLESHEET = """
QMainWindow {
    background-color: #0d1117;
    color: #e6edf3;
}

QWidget {
    background-color: #0d1117;
    color: #e6edf3;
    font-family: 'Segoe UI', 'SF Pro Display', -apple-system, sans-serif;
    font-size: 13px;
}

QLabel {
    color: #e6edf3;
    background-color: transparent;
}

QPushButton {
    background-color: #21262d;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 8px 16px;
    min-height: 28px;
    font-size: 13px;
}

QPushButton:hover {
    background-color: #30363d;
    border: 1px solid #8b949e;
}

QPushButton:pressed {
    background-color: #1c2128;
}

QPushButton:disabled {
    background-color: #21262d;
    color: #484f58;
    border: 1px solid #21262d;
}

QPushButton#primaryButton {
    background-color: #238636;
    color: #ffffff;
    border: 1px solid #238636;
}

QPushButton#primaryButton:hover {
    background-color: #2ea043;
    border: 1px solid #2ea043;
}

QPushButton#dangerButton {
    background-color: #da3633;
    color: #ffffff;
    border: 1px solid #da3633;
}

QPushButton#dangerButton:hover {
    background-color: #f85149;
    border: 1px solid #f85149;
}

QPushButton#warningButton {
    background-color: #9e6a03;
    color: #ffffff;
    border: 1px solid #9e6a03;
}

QPushButton#warningButton:hover {
    background-color: #bb8009;
    border: 1px solid #bb8009;
}

QTabWidget::pane {
    background-color: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 8px;
}

QTabBar::tab {
    background-color: #0d1117;
    color: #8b949e;
    border: 1px solid #30363d;
    border-bottom: none;
    border-top-left-radius: 6px;
    border-top-right-radius: 6px;
    padding: 10px 24px;
    margin-right: 2px;
    font-size: 13px;
}

QTabBar::tab:selected {
    background-color: #161b22;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-bottom: 1px solid #161b22;
}

QTabBar::tab:hover:!selected {
    background-color: #21262d;
    color: #e6edf3;
}

QGroupBox {
    background-color: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    margin-top: 12px;
    padding: 16px;
    font-size: 13px;
}

QGroupBox::title {
    color: #e6edf3;
    subcontrol-origin: margin;
    subcontrol-position: top left;
    padding: 0 8px;
    font-size: 14px;
    font-weight: bold;
}

QCheckBox {
    color: #e6edf3;
    spacing: 8px;
    font-size: 13px;
}

QCheckBox::indicator {
    width: 18px;
    height: 18px;
    border: 1px solid #30363d;
    border-radius: 4px;
    background-color: #21262d;
}

QCheckBox::indicator:checked {
    background-color: #238636;
    border: 1px solid #238636;
}

QSpinBox, QDoubleSpinBox {
    background-color: #21262d;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 6px;
    min-height: 24px;
}

QSpinBox:focus, QDoubleSpinBox:focus {
    border: 1px solid #58a6ff;
}

QSpinBox::up-button, QDoubleSpinBox::up-button {
    background-color: #30363d;
    border-left: 1px solid #30363d;
    border-radius: 0 4px 0 0;
}

QSpinBox::down-button, QDoubleSpinBox::down-button {
    background-color: #30363d;
    border-left: 1px solid #30363d;
    border-radius: 0 0 4px 0;
}

QLineEdit, QTextEdit, QPlainTextEdit {
    background-color: #21262d;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 8px;
    font-size: 13px;
    selection-background-color: #58a6ff;
}

QLineEdit:focus, QTextEdit:focus, QPlainTextEdit:focus {
    border: 1px solid #58a6ff;
}

QTextEdit#resultsDisplay {
    background-color: #0d1117;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 12px;
    font-family: 'Cascadia Code', 'Fira Code', 'Consolas', monospace;
    font-size: 12px;
}

QProgressBar {
    background-color: #21262d;
    border: none;
    border-radius: 4px;
    height: 6px;
    text-align: center;
}

QProgressBar::chunk {
    background-color: #238636;
    border-radius: 4px;
}

QStatusBar {
    background-color: #161b22;
    color: #8b949e;
    border-top: 1px solid #30363d;
}

QToolBar {
    background-color: #161b22;
    border: none;
    spacing: 4px;
    padding: 4px;
}

QToolBar::separator {
    background-color: #30363d;
    width: 1px;
    margin: 4px 8px;
}

QMenuBar {
    background-color: #161b22;
    color: #e6edf3;
    border-bottom: 1px solid #30363d;
}

QMenuBar::item:selected {
    background-color: #30363d;
}

QMenu {
    background-color: #161b22;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 6px;
}

QMenu::item:selected {
    background-color: #30363d;
}

QComboBox {
    background-color: #21262d;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 6px 12px;
}

QComboBox:hover {
    border: 1px solid #8b949e;
}

QComboBox::drop-down {
    border: none;
    background-color: #21262d;
}

QComboBox QAbstractItemView {
    background-color: #161b22;
    color: #e6edf3;
    border: 1px solid #30363d;
    selection-background-color: #30363d;
}

QListWidget, QTreeWidget {
    background-color: #21262d;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 4px;
}

QListWidget::item, QTreeWidget::item {
    padding: 6px 8px;
    border-radius: 4px;
}

QListWidget::item:selected, QTreeWidget::item:selected {
    background-color: #30363d;
}

QListWidget::item:hover:!selected, QTreeWidget::item:hover:!selected {
    background-color: #21262d;
}

QScrollBar:vertical {
    background-color: #0d1117;
    width: 10px;
    margin: 0;
}

QScrollBar::handle:vertical {
    background-color: #30363d;
    border-radius: 5px;
    min-height: 30px;
}

QScrollBar::handle:vertical:hover {
    background-color: #484f58;
}

QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
    height: 0;
}

QScrollBar:horizontal {
    background-color: #0d1117;
    height: 10px;
    margin: 0;
}

QScrollBar::handle:horizontal {
    background-color: #30363d;
    border-radius: 5px;
    min-width: 30px;
}

QScrollBar::handle:horizontal:hover {
    background-color: #484f58;
}

QDockWidget {
    background-color: #161b22;
    color: #e6edf3;
    border: 1px solid #30363d;
    border-radius: 8px;
    titlebar-close-icon: url(close.svg);
}

QDockWidget::title {
    background-color: #161b22;
    padding: 8px;
    border-bottom: 1px solid #30363d;
}

QSplitter::handle {
    background-color: #30363d;
}

QSplitter::handle:horizontal {
    width: 2px;
}

QSplitter::handle:vertical {
    height: 2px;
}

QHeaderView::section {
    background-color: #21262d;
    color: #e6edf3;
    padding: 8px;
    border: none;
    border-bottom: 1px solid #30363d;
}

QTableWidget {
    background-color: #21262d;
    color: #e6edf3;
    grid-color: #30363d;
    border: 1px solid #30363d;
    border-radius: 6px;
}

QTableWidget::item {
    padding: 6px 8px;
}

QTableWidget::item:selected {
    background-color: #30363d;
}

QRadioButton {
    color: #e6edf3;
    spacing: 8px;
}

QRadioButton::indicator {
    width: 18px;
    height: 18px;
    border: 1px solid #30363d;
    border-radius: 9px;
    background-color: #21262d;
}

QRadioButton::indicator:checked {
    background-color: #238636;
    border: 1px solid #238636;
}
"""

SIM_BUTTON_STYLES = """
QPushButton#sim_thermal {
    background-color: #ff6b6b;
    color: white;
    border: 1px solid #ff6b6b;
}
QPushButton#sim_thermal:hover {
    background-color: #ff8787;
}

QPushButton#sim_crosstalk {
    background-color: #9b59b6;
    color: white;
    border: 1px solid #9b59b6;
}
QPushButton#sim_crosstalk:hover {
    background-color: #af7ac5;
}

QPushButton#sim_emi {
    background-color: #e67e22;
    color: white;
    border: 1px solid #e67e22;
}
QPushButton#sim_emi:hover {
    background-color: #f39c4d;
}

QPushButton#sim_power {
    background-color: #3498db;
    color: white;
    border: 1px solid #3498db;
}
QPushButton#sim_power:hover {
    background-color: #5dade2;
}

QPushButton#sim_loops {
    background-color: #2ecc71;
    color: white;
    border: 1px solid #2ecc71;
}
QPushButton#sim_loops:hover {
    background-color: #58d68d;
}

QPushButton#sim_df {
    background-color: #1abc9c;
    color: white;
    border: 1px solid #1abc9c;
}
QPushButton#sim_df:hover {
    background-color: #48c9b0;
}

QPushButton#sim_thd {
    background-color: #e74c3c;
    color: white;
    border: 1px solid #e74c3c;
}
QPushButton#sim_thd:hover {
    background-color: #ec7063;
}

QPushButton#sim_psrr {
    background-color: #7950f2;
    color: white;
    border: 1px solid #7950f2;
}
QPushButton#sim_psrr:hover {
    background-color: #9775fa;
}

QPushButton#sim_stability {
    background-color: #fdcb6e;
    color: #333;
    border: 1px solid #fdcb6e;
}
QPushButton#sim_stability:hover {
    background-color: #fdd835;
}

QPushButton#run_all {
    background-color: qlineargradient(x1:0, y1:0, x2:1, y2:1, stop:0 #238636, stop:1 #2ea043);
    color: white;
    border: 1px solid #238636;
    font-weight: bold;
    font-size: 14px;
    padding: 12px 24px;
}
QPushButton#run_all:hover {
    background-color: qlineargradient(x1:0, y1:0, x2:1, y2:1, stop:0 #2ea043, stop:1 #3fb950);
}
"""
