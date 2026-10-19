#pragma once

#include "editor.h"

#include <functional>

class QCheckBox;
class QComboBox;
class QLabel;
class QLineEdit;
class QPushButton;
class RjBackend;

/// Datei → Exportieren: Struktur und Ziele als JSON (remotejoe) oder CSV (gängiges Spaltenformat), auf Wunsch mit
/// Kennwörtern und als verschlüsseltes ZIP (AES-256). Mit Kennwörtern ist beim JSON das ZIP Pflicht.
class ExportTab : public ToolTab
{
	Q_OBJECT
public:
	ExportTab(RjBackend* backend, std::function<QStringList()> selectedIds, QWidget* parent = nullptr);
	void wipeSecrets() override;

private:
	void update();
	void run();
	QString suggestedPath() const;
	RjBackend* m_backend;
	std::function<QStringList()> m_selectedIds;
	QComboBox* m_format;
	QComboBox* m_scope;
	QCheckBox* m_passwords;
	QCheckBox* m_zip;
	QLineEdit* m_zipPassword;
	QLineEdit* m_zipPassword2;
	QLineEdit* m_path;
	QLabel* m_warning;
	QLabel* m_result;
	QPushButton* m_go;
	bool m_pathEdited = false;
};
