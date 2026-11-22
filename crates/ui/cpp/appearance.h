#pragma once

#include "editor.h"

#include <QtCore/QJsonObject>
#include <QtCore/QList>
#include <QtCore/QMap>

class QCheckBox;
class QComboBox;
class QGridLayout;
class QGroupBox;
class QLabel;
class QLineEdit;
class QPushButton;
class QWidget;
class RjBackend;
class PaletteTile;

/// Extras → Darstellung: Modus (automatisch nach System mit je einer Palette für hell und dunkel,
/// oder eine feste Palette), Paletten als Kacheln mit Farbvorschau, eigene Palette mit
/// Farbwählern und Fokusrahmen. Änderungen gelten sofort und werden gespeichert.
class AppearanceTab : public ToolTab
{
	Q_OBJECT
public:
	explicit AppearanceTab(RjBackend* backend, QWidget* parent = nullptr);
	void reload();

signals:
	/// Schema oder Fokusrahmen wurden geändert (Oberfläche neu einfärben).
	void appearanceChanged();

protected:
	void resizeEvent(QResizeEvent* e) override;

private:
	/// Kacheln im Raster anordnen; Spaltenzahl nach verfügbarer Breite (auch in der schmaleren Karte).
	void placeTiles();
	int m_columns = 0;
	void apply(const QJsonObject& change);
	void applyFromControls();
	void tileClicked(const QString& id, bool dark);
	void openEditor();
	void pickColor(const QString& key);
	void saveEditor();
	void updateEditorPreview();
	RjBackend* m_backend;
	QComboBox* m_mode;
	QComboBox* m_light;
	QComboBox* m_dark;
	QComboBox* m_fixed;
	QWidget* m_systemRow;
	QWidget* m_fixedRow;
	QCheckBox* m_focus;
	QComboBox* m_treeSize;
	QComboBox* m_iconStyle;
	QCheckBox* m_ownIcons;
	QWidget* m_iconBox;
	QMap<QString, QPushButton*> m_iconButtons;
	QMap<QString, QString> m_iconColors;
	void applyIcons();
	QGridLayout* m_tiles;
	QList<PaletteTile*> m_tileList;
	QLabel* m_notes;
	QGroupBox* m_editor;
	QLineEdit* m_editName;
	QCheckBox* m_editDark;
	QMap<QString, QPushButton*> m_colorButtons;
	QMap<QString, QString> m_editColors;
	PaletteTile* m_editPreview;
	QString m_dir;
	QJsonObject m_state;
	bool m_loading = false;
};
