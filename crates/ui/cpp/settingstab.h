#pragma once

// Extras → Einstellungen: ein Reiter im Kartenstil wie die Einstellungen eines Ziels. Kopfkarte, links die
// Bereiche (Sperre und Zwischenablage, Tastatur, Terminal, Darstellung, SNMP-Geräte), rechts die Karte des
// gewählten Bereichs. Alle Änderungen gelten sofort und werden gespeichert; es gibt nichts zu speichern oder
// zu verwerfen.

#include "editor.h"

#include <QtCore/QMap>
#include <QtCore/QString>

class AppearanceTab;
class QCheckBox;
class QComboBox;
class QLabel;
class QLineEdit;
class QPushButton;
class QSpinBox;
class QStackedWidget;
class QVBoxLayout;
class RjBackend;
class SnmpTab;
namespace rjui
{
class ClickRow;
}

class SettingsTab : public ToolTab
{
	Q_OBJECT
public:
	explicit SettingsTab(RjBackend* backend, QWidget* parent = nullptr);
	/// Bereich zeigen: lock, keys, terminal, appearance, snmp.
	void showSection(const QString& key);
	QString currentSection() const { return m_current; }
	void wipeSecrets() override;
	/// Werte neu lesen (nach Entsperren oder Änderungen von außen).
	void reload();
	/// Stand der Sicherung neu zeigen (nach einer Sicherung im Hintergrund).
	void reloadBackup();

signals:
	/// Farbschema oder Fokusrahmen geändert (Oberfläche neu einfärben).
	void appearanceChanged();
	/// Strg+Umschalt+W: neuer Modus (outside, always, never).
	void closeTabKeyChanged(const QString& mode);
	/// Zusatzfunktionen ein- oder ausgeschaltet (z. B. Aruba-Werkzeuge).
	void featuresChanged();

private:
	struct Area
	{
		rjui::ClickRow* row;
		QLabel* summary;
		int page;
	};
	void addArea(const QString& key, const QString& title, QWidget* page, bool scroll);
	QWidget* pageLock();
	QWidget* pageKeys();
	QWidget* pageFeatures();
	QWidget* pageBackup();
	void saveBackup();
	void refreshSummaries();
	QString edit(const QJsonObject& cmd);

	RjBackend* m_backend;
	QVBoxLayout* m_areaList = nullptr;
	QStackedWidget* m_pages = nullptr;
	QLabel* m_pageTitle = nullptr;
	QMap<QString, Area> m_areas;
	QMap<QString, QString> m_titles;
	QString m_current;
	bool m_loading = false;
	// Sperre und Zwischenablage
	QComboBox* m_idle = nullptr;
	QCheckBox* m_disconnect = nullptr;
	QSpinBox* m_clipSecs = nullptr;
	// Tastatur
	QCheckBox* m_keyCapture = nullptr;
	QComboBox* m_closeTab = nullptr;
	QCheckBox* m_aruba = nullptr;
	// Sicherung
	QCheckBox* m_bkEnabled = nullptr;
	QSpinBox* m_bkLast = nullptr;
	QSpinBox* m_bkDays = nullptr;
	QSpinBox* m_bkUpdate = nullptr;
	QCheckBox* m_bkSecond = nullptr;
	QLineEdit* m_bkDir = nullptr;
	QLabel* m_bkLocal = nullptr;
	QLabel* m_bkState = nullptr;
	QLabel* m_bkKeyFile = nullptr;
	QPushButton* m_bkNow = nullptr;
	QString m_bkLocalDir;
	// Eingebettete Bereiche
	AppearanceTab* m_appearance = nullptr;
	SnmpTab* m_snmp = nullptr;
};
