#pragma once
// Mehrzeiliges Textfeld mit sicherem Einfügen. Qt 6.4 (Ubuntu 24.04) stürzt unter Wayland beim Einfügen aus der
// Zwischenablage eines anderen Programms in QWidgetTextControl::insertFromMimeData ab. Dieses Feld umgeht den Weg:
// Beim Einfügen aus der Zwischenablage liest es den Text frisch aus der Zwischenablage (als reinen Text); ist sie
// leer oder ohne Text, passiert nichts. Ziehen und Ablegen aus anderen Quellen fügt ebenfalls nur reinen Text ein.

#include <QtCore/QMimeData>
#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>
#include <QtWidgets/QPlainTextEdit>

class RjPlainTextEdit : public QPlainTextEdit
{
public:
	using QPlainTextEdit::QPlainTextEdit;

	/// Text, der beim Einfügen von `source` eingesetzt würde (leer = nichts einfügen); für Tests.
	static QString pasteText(const QMimeData* source)
	{
		QClipboard* cb = QGuiApplication::clipboard();
		if (!source)
			return QString();
		if (cb && source == cb->mimeData(QClipboard::Clipboard))
			return cb->text(QClipboard::Clipboard);
		if (cb && cb->supportsSelection() && source == cb->mimeData(QClipboard::Selection))
			return cb->text(QClipboard::Selection);
		return source->hasText() ? source->text() : QString();
	}

protected:
	bool canInsertFromMimeData(const QMimeData* source) const override
	{
		return source && source->hasText();
	}

	void insertFromMimeData(const QMimeData* source) override
	{
		const QString text = pasteText(source);
		if (!text.isEmpty())
			insertPlainText(text);
	}
};
