#pragma once

#include "editor.h"

class QCheckBox;
class QComboBox;
class QLabel;
class QPushButton;
class RjBackend;

/// Extras → Darstellung: Farbschema (System, Hell, Dunkel, eigene Paletten) und Fokusrahmen.
/// Änderungen gelten sofort und werden gespeichert.
class AppearanceTab : public ToolTab
{
	Q_OBJECT
public:
	explicit AppearanceTab(RjBackend* backend, QWidget* parent = nullptr);
	void reload();

signals:
	/// Schema oder Fokusrahmen wurden geändert (Oberfläche neu einfärben).
	void appearanceChanged();

private:
	void apply();
	RjBackend* m_backend;
	QComboBox* m_scheme;
	QCheckBox* m_focus;
	QLabel* m_notes;
	QString m_dir;
	bool m_loading = false;
};
