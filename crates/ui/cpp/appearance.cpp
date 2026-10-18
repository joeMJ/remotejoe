#include "appearance.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QUrl>
#include <QtGui/QDesktopServices>
#include <QtGui/QMouseEvent>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QColorDialog>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QGridLayout>
#include <QtWidgets/QGroupBox>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QScrollArea>
#include <QtWidgets/QVBoxLayout>

#include <functional>

namespace {

QJsonObject parse(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}

/// Rollen der Palette in der Reihenfolge des Editors (Schlüssel, Textschlüssel).
const char* const kRoles[][2] = {
	{ "window", "appearance.role.window" },     { "base", "appearance.role.base" },
	{ "alternate", "appearance.role.alternate" }, { "text", "appearance.role.text" },
	{ "muted", "appearance.role.muted" },       { "highlight", "appearance.role.highlight" },
	{ "highlight_text", "appearance.role.highlight_text" }, { "button", "appearance.role.button" },
	{ "link", "appearance.role.link" },         { "warning", "appearance.role.warning" },
	{ "error", "appearance.role.error" },       { "ok", "appearance.role.ok" },
};

} // namespace

/// Kachel mit Farbvorschau einer Palette: Fensterrahmen, Grundfläche mit Text, Auswahlbalken und
/// Punkte für Warnung, Fehler, OK. Ein Klick wählt die Palette.
class PaletteTile : public QFrame
{
public:
	PaletteTile(QWidget* parent)
	  : QFrame(parent)
	{
		setFixedSize(200, 118);
		setCursor(Qt::PointingHandCursor);
		setFocusPolicy(Qt::StrongFocus);
	}
	void set(const QString& name, const QJsonObject& colors, const QString& badge)
	{
		m_name = name;
		m_colors = colors;
		m_badge = badge;
		setAccessibleName(badge.isEmpty() ? name : name + QStringLiteral(" – ") + badge);
		setToolTip(accessibleName());
		update();
	}
	std::function<void()> clicked;

protected:
	void mouseReleaseEvent(QMouseEvent* e) override
	{
		if (e->button() == Qt::LeftButton && rect().contains(e->position().toPoint()) && clicked)
			clicked();
	}
	void keyPressEvent(QKeyEvent* e) override
	{
		if ((e->key() == Qt::Key_Space || e->key() == Qt::Key_Return || e->key() == Qt::Key_Enter) && clicked)
			clicked();
		else
			QFrame::keyPressEvent(e);
	}
	void paintEvent(QPaintEvent*) override
	{
		auto col = [this](const char* k) { return QColor(m_colors.value(QString::fromLatin1(k)).toString()); };
		QPainter p(this);
		p.setRenderHint(QPainter::Antialiasing);
		const QRectF r = QRectF(rect()).adjusted(2, 2, -2, -2);
		const bool active = !m_badge.isEmpty();
		// Rahmen: aktiv in der Auswahlfarbe der Oberfläche, sonst dezent.
		p.setPen(QPen(active ? palette().color(QPalette::Highlight) : palette().color(QPalette::Mid), active ? 3 : 1));
		p.setBrush(col("window"));
		p.drawRoundedRect(r, 8, 8);
		const QRectF base(r.left() + 10, r.top() + 10, r.width() - 20, 52);
		p.setPen(Qt::NoPen);
		p.setBrush(col("base"));
		p.drawRoundedRect(base, 5, 5);
		QFont f = font();
		f.setPointSizeF(f.pointSizeF() * 0.9);
		p.setFont(f);
		p.setPen(col("text"));
		p.drawText(base.adjusted(8, 4, -4, -26), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("Aa Text"));
		p.setPen(col("muted"));
		p.drawText(base.adjusted(70, 4, -4, -26), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("neben"));
		const QRectF sel(base.left() + 6, base.top() + 30, base.width() - 12, 16);
		p.setPen(Qt::NoPen);
		p.setBrush(col("highlight"));
		p.drawRoundedRect(sel, 3, 3);
		p.setPen(col("highlight_text"));
		p.drawText(sel.adjusted(6, 0, 0, 0), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("Auswahl"));
		// Warnung, Fehler, OK als Punkte rechts in der ersten Zeile.
		qreal x = base.right() - 10;
		for (const char* k : { "ok", "error", "warning" })
		{
			p.setPen(Qt::NoPen);
			p.setBrush(col(k));
			p.drawEllipse(QPointF(x, base.top() + 15), 4.5, 4.5);
			x -= 13;
		}
		QFont nf = font();
		nf.setBold(true);
		p.setFont(nf);
		p.setPen(col("text"));
		p.drawText(QRectF(r.left() + 10, base.bottom() + 2, r.width() - 20, 22), Qt::AlignLeft | Qt::AlignVCenter,
		           p.fontMetrics().elidedText(m_name, Qt::ElideRight, int(r.width()) - 20));
		if (active)
		{
			p.setFont(f);
			p.setPen(col("muted"));
			p.drawText(QRectF(r.left() + 10, base.bottom() + 22, r.width() - 20, 18), Qt::AlignLeft | Qt::AlignVCenter, m_badge);
		}
		if (hasFocus())
		{
			p.setPen(QPen(palette().color(QPalette::Highlight), 1, Qt::DotLine));
			p.setBrush(Qt::NoBrush);
			p.drawRoundedRect(r.adjusted(4, 4, -4, -4), 6, 6);
		}
	}

private:
	QString m_name;
	QJsonObject m_colors;
	QString m_badge;
};

AppearanceTab::AppearanceTab(RjBackend* backend, QWidget* parent)
  : ToolTab(parent)
  , m_backend(backend)
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	setObjectName(QStringLiteral("appearanceTab"));
	auto* outer = new QVBoxLayout(this);
	outer->setContentsMargins(0, 0, 0, 0);
	auto* scroll = new QScrollArea(this);
	scroll->setWidgetResizable(true);
	scroll->setFrameShape(QFrame::NoFrame);
	outer->addWidget(scroll);
	auto* page = new QWidget(scroll);
	scroll->setWidget(page);
	auto* lay = new QVBoxLayout(page);

	auto* form = new QFormLayout;
	m_mode = new QComboBox(page);
	m_mode->addItem(T("appearance.mode.system"), QStringLiteral("system"));
	m_mode->addItem(T("appearance.mode.fixed"), QStringLiteral("fixed"));
	form->addRow(T("appearance.mode"), m_mode);
	m_systemRow = new QWidget(page);
	auto* sr = new QFormLayout(m_systemRow);
	sr->setContentsMargins(0, 0, 0, 0);
	m_light = new QComboBox(m_systemRow);
	m_dark = new QComboBox(m_systemRow);
	sr->addRow(T("appearance.light"), m_light);
	sr->addRow(T("appearance.dark"), m_dark);
	form->addRow(QString(), m_systemRow);
	m_fixedRow = new QWidget(page);
	auto* fr = new QFormLayout(m_fixedRow);
	fr->setContentsMargins(0, 0, 0, 0);
	m_fixed = new QComboBox(m_fixedRow);
	fr->addRow(T("appearance.fixed"), m_fixed);
	form->addRow(QString(), m_fixedRow);
	m_focus = new QCheckBox(T("appearance.focus"), page);
	form->addRow(QString(), m_focus);
	m_treeSize = new QComboBox(page);
	for (const char* s : { "klein", "mittel", "gross" })
		m_treeSize->addItem(T((QByteArray("appearance.tree_size.") + s).constData()), QString::fromLatin1(s));
	form->addRow(T("appearance.tree_size"), m_treeSize);
	// Symbolfarben: dem Theme folgen oder eigene Farbe je Art.
	m_iconStyle = new QComboBox(page);
	for (const char* s : { "einheitlich", "je_typ" })
		m_iconStyle->addItem(T((QByteArray("appearance.icons.style.") + s).constData()), QString::fromLatin1(s));
	form->addRow(T("appearance.icons"), m_iconStyle);
	m_ownIcons = new QCheckBox(T("appearance.icons.own"), page);
	form->addRow(QString(), m_ownIcons);
	m_iconBox = new QWidget(page);
	auto* ig = new QGridLayout(m_iconBox);
	ig->setContentsMargins(0, 0, 0, 0);
	int ii = 0;
	for (const char* k : { "rdp", "ssh", "vnc", "web", "psrp", "proxmox", "hyperv", "macro", "folder" })
	{
		const QString key = QString::fromLatin1(k);
		auto* b = new QPushButton(T((QByteArray("appearance.icons.kind.") + k).constData()), m_iconBox);
		b->setAccessibleName(b->text());
		m_iconButtons.insert(key, b);
		ig->addWidget(b, ii / 3, ii % 3);
		++ii;
		connect(b, &QPushButton::clicked, this, [this, key] {
			const QColor c = QColorDialog::getColor(QColor(m_iconColors.value(key)), this, m_iconButtons.value(key)->accessibleName());
			if (!c.isValid())
				return;
			m_iconColors.insert(key, c.name());
			applyIcons();
		});
	}
	form->addRow(QString(), m_iconBox);
	lay->addLayout(form);

	auto* help = new QLabel(T("appearance.tiles.help"), page);
	help->setWordWrap(true);
	lay->addWidget(help);
	auto* tilesBox = new QWidget(page);
	m_tiles = new QGridLayout(tilesBox);
	m_tiles->setContentsMargins(0, 0, 0, 0);
	m_tiles->setHorizontalSpacing(10);
	m_tiles->setVerticalSpacing(10);
	auto* tilesRow = new QHBoxLayout;
	tilesRow->addWidget(tilesBox);
	tilesRow->addStretch(1);
	lay->addLayout(tilesRow);

	auto* row = new QHBoxLayout;
	auto* create = new QPushButton(T("appearance.create"), page);
	auto* open = new QPushButton(T("appearance.folder"), page);
	auto* reloadButton = new QPushButton(T("appearance.reload"), page);
	row->addWidget(create);
	row->addWidget(open);
	row->addWidget(reloadButton);
	row->addStretch(1);
	lay->addLayout(row);

	// Eigene Palette mit Farbwählern (statt JSON von Hand).
	m_editor = new QGroupBox(T("appearance.editor"), page);
	auto* ev = new QHBoxLayout(m_editor);
	auto* ef = new QFormLayout;
	m_editName = new QLineEdit(m_editor);
	ef->addRow(T("appearance.editor.name"), m_editName);
	m_editDark = new QCheckBox(T("appearance.editor.dark"), m_editor);
	ef->addRow(QString(), m_editDark);
	auto* grid = new QGridLayout;
	int i = 0;
	for (const auto& role : kRoles)
	{
		const QString key = QString::fromLatin1(role[0]);
		auto* b = new QPushButton(m_editor);
		b->setMinimumWidth(110);
		b->setAccessibleName(T(role[1]));
		m_colorButtons.insert(key, b);
		grid->addWidget(new QLabel(T(role[1]), m_editor), i / 2, (i % 2) * 2);
		grid->addWidget(b, i / 2, (i % 2) * 2 + 1);
		connect(b, &QPushButton::clicked, this, [this, key] { pickColor(key); });
		++i;
	}
	ef->addRow(grid);
	auto* eb = new QHBoxLayout;
	auto* save = new QPushButton(T("appearance.editor.save"), m_editor);
	auto* cancel = new QPushButton(T("appearance.editor.cancel"), m_editor);
	eb->addWidget(save);
	eb->addWidget(cancel);
	eb->addStretch(1);
	ef->addRow(eb);
	ev->addLayout(ef, 1);
	m_editPreview = new PaletteTile(m_editor);
	ev->addWidget(m_editPreview, 0, Qt::AlignTop);
	m_editor->hide();
	lay->addWidget(m_editor);

	m_notes = new QLabel(page);
	m_notes->setWordWrap(true);
	m_notes->setTextInteractionFlags(Qt::TextSelectableByMouse);
	lay->addWidget(m_notes);
	auto* old = new QLabel(T("appearance.help"), page);
	old->setWordWrap(true);
	old->setEnabled(false);
	lay->addWidget(old);
	lay->addStretch(1);

	for (QComboBox* c : { m_mode, m_light, m_dark, m_fixed })
		connect(c, &QComboBox::currentIndexChanged, this, [this] { applyFromControls(); });
	connect(m_focus, &QCheckBox::toggled, this, [this] { applyFromControls(); });
	connect(m_treeSize, &QComboBox::currentIndexChanged, this, [this] {
		if (!m_loading)
			apply({ { QStringLiteral("tree_size"), m_treeSize->currentData().toString() } });
	});
	connect(m_iconStyle, &QComboBox::currentIndexChanged, this, [this] {
		if (!m_loading)
			apply({ { QStringLiteral("icon_style"), m_iconStyle->currentData().toString() } });
	});
	connect(m_ownIcons, &QCheckBox::toggled, this, [this](bool on) {
		if (m_loading)
			return;
		if (!on)
			m_iconColors.clear();
		applyIcons();
	});
	connect(reloadButton, &QPushButton::clicked, this, [this] { reload(); });
	connect(create, &QPushButton::clicked, this, [this] { openEditor(); });
	connect(save, &QPushButton::clicked, this, [this] { saveEditor(); });
	connect(cancel, &QPushButton::clicked, m_editor, &QWidget::hide);
	connect(m_editName, &QLineEdit::textChanged, this, [this] { updateEditorPreview(); });
	connect(m_editDark, &QCheckBox::toggled, this, [this] { updateEditorPreview(); });
	connect(open, &QPushButton::clicked, this, [this] {
		if (m_dir.isEmpty())
			return;
		QDir().mkpath(m_dir);
		QDesktopServices::openUrl(QUrl::fromLocalFile(m_dir));
		reload();
	});
	reload();
}

void AppearanceTab::reload()
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	m_state = parse(m_backend->appearanceJson());
	m_loading = true;
	const QJsonArray palettes = m_state.value(QStringLiteral("palettes")).toArray();
	const QString scheme = m_state.value(QStringLiteral("scheme")).toString();
	const QString light = m_state.value(QStringLiteral("light_theme")).toString();
	const QString dark = m_state.value(QStringLiteral("dark_theme")).toString();
	const bool system = scheme == QLatin1String("system");
	m_mode->setCurrentIndex(system ? 0 : 1);
	// Auswahllisten: hell zuerst für den hellen Modus, dunkel zuerst für den dunklen.
	auto fill = [&](QComboBox* c, bool withDesktop, int darkFirst, const QString& current) {
		c->clear();
		if (withDesktop)
			c->addItem(T("appearance.desktop"), QStringLiteral("system"));
		for (int pass = 0; pass < 2; ++pass)
			for (const QJsonValue& v : palettes)
			{
				const QJsonObject o = v.toObject();
				const bool d = o.value(QStringLiteral("dark")).toBool();
				if (darkFirst >= 0 && (pass == 0) != (d == bool(darkFirst)))
					continue;
				if (darkFirst < 0 && pass == 1)
					continue;
				c->addItem(o.value(QStringLiteral("name")).toString() + (d ? T("appearance.suffix.dark") : T("appearance.suffix.light")), o.value(QStringLiteral("id")).toString());
			}
		c->setCurrentIndex(qMax(0, c->findData(current)));
	};
	fill(m_light, true, 0, light);
	fill(m_dark, true, 1, dark);
	fill(m_fixed, false, -1, system ? QStringLiteral("hell") : scheme);
	m_systemRow->setVisible(system);
	m_fixedRow->setVisible(!system);
	m_focus->setChecked(m_state.value(QStringLiteral("focus_frame")).toBool(true));
	m_treeSize->setCurrentIndex(qMax(0, m_treeSize->findData(m_state.value(QStringLiteral("tree_size")).toString(QStringLiteral("mittel")))));
	m_iconStyle->setCurrentIndex(qMax(0, m_iconStyle->findData(m_state.value(QStringLiteral("icon_style")).toString(QStringLiteral("einheitlich")))));
	{
		// Eigene Farben aus der Einstellung, sonst die Farben des Themes anzeigen.
		const QJsonObject own = m_state.value(QStringLiteral("icon_colors")).toObject();
		const QJsonObject theme = parse(m_backend->paletteJson()).value(QStringLiteral("icons")).toObject();
		m_iconColors.clear();
		for (auto it = own.begin(); it != own.end(); ++it)
			m_iconColors.insert(it.key(), it.value().toString());
		m_ownIcons->setChecked(!own.isEmpty());
		m_iconBox->setEnabled(!own.isEmpty());
		for (auto it = m_iconButtons.begin(); it != m_iconButtons.end(); ++it)
		{
			const QString c = m_iconColors.value(it.key(), theme.value(it.key()).toString());
			it.value()->setStyleSheet(QStringLiteral("QPushButton { border-left: 14px solid %1; padding-left: 8px; }").arg(c));
		}
	}
	m_dir = m_state.value(QStringLiteral("dir")).toString();

	// Kacheln neu aufbauen; aktive Paletten tragen ein Kennzeichen.
	for (PaletteTile* t : m_tileList)
		t->deleteLater();
	m_tileList.clear();
	const int columns = 4;
	int n = 0;
	for (const QJsonValue& v : palettes)
	{
		const QJsonObject o = v.toObject();
		const QString id = o.value(QStringLiteral("id")).toString();
		const bool d = o.value(QStringLiteral("dark")).toBool();
		QString badge;
		if (system && id == light)
			badge = T("appearance.badge.light");
		if (system && id == dark)
			badge = badge.isEmpty() ? T("appearance.badge.dark") : T("appearance.badge.both");
		if (!system && id == scheme)
			badge = T("appearance.badge.fixed");
		auto* tile = new PaletteTile(m_tiles->parentWidget());
		tile->set(o.value(QStringLiteral("name")).toString() + (o.value(QStringLiteral("own")).toBool() ? T("appearance.suffix.own") : QString()),
		          o.value(QStringLiteral("colors")).toObject(), badge);
		tile->clicked = [this, id, d] { tileClicked(id, d); };
		m_tiles->addWidget(tile, n / columns, n % columns);
		m_tileList << tile;
		++n;
	}
	QStringList notes;
	for (const QJsonValue& v : m_state.value(QStringLiteral("notes")).toArray())
		notes << v.toString();
	m_notes->setText(notes.join(QLatin1Char('\n')));
	m_notes->setVisible(!notes.isEmpty());
	m_loading = false;
}

void AppearanceTab::apply(const QJsonObject& change)
{
	const QJsonObject r = parse(m_backend->setAppearance(QString::fromUtf8(QJsonDocument(change).toJson(QJsonDocument::Compact))));
	if (r.contains(QStringLiteral("error")))
		emit status(r.value(QStringLiteral("error")).toString());
	emit appearanceChanged();
	reload();
}

void AppearanceTab::applyFromControls()
{
	if (m_loading)
		return;
	QJsonObject o;
	const bool system = m_mode->currentData().toString() == QLatin1String("system");
	o.insert(QStringLiteral("scheme"), system ? QStringLiteral("system") : m_fixed->currentData().toString());
	o.insert(QStringLiteral("light_theme"), m_light->currentData().toString());
	o.insert(QStringLiteral("dark_theme"), m_dark->currentData().toString());
	o.insert(QStringLiteral("focus_frame"), m_focus->isChecked());
	apply(o);
}

void AppearanceTab::tileClicked(const QString& id, bool dark)
{
	// Automatisch: Die Kachel gilt für den Modus, zu dem sie passt; fest: sofort als Palette.
	if (m_state.value(QStringLiteral("scheme")).toString() == QLatin1String("system"))
		apply({ { dark ? QStringLiteral("dark_theme") : QStringLiteral("light_theme"), id } });
	else
		apply({ { QStringLiteral("scheme"), id } });
}

void AppearanceTab::openEditor()
{
	// Vorlage: die gerade wirksamen Farben.
	const QJsonObject p = parse(m_backend->paletteJson());
	m_editColors.clear();
	for (const auto& role : kRoles)
		m_editColors.insert(QString::fromLatin1(role[0]), p.value(QString::fromLatin1(role[0])).toString());
	m_editDark->setChecked(p.value(QStringLiteral("dark")).toBool());
	m_editName->setText(m_backend->text(QStringLiteral("appearance.editor.default")));
	updateEditorPreview();
	m_editor->show();
	m_editName->setFocus();
	m_editName->selectAll();
}

void AppearanceTab::applyIcons()
{
	// Eigene Farben: alle Arten festhalten (fehlende mit der Farbe des Themes), sonst leer.
	QJsonObject o;
	if (m_ownIcons->isChecked())
	{
		const QJsonObject theme = parse(m_backend->paletteJson()).value(QStringLiteral("icons")).toObject();
		for (auto it = m_iconButtons.cbegin(); it != m_iconButtons.cend(); ++it)
			o.insert(it.key(), m_iconColors.value(it.key(), theme.value(it.key()).toString()));
	}
	apply({ { QStringLiteral("icon_colors"), o } });
}

void AppearanceTab::pickColor(const QString& key)
{
	const QColor c = QColorDialog::getColor(QColor(m_editColors.value(key)), this, m_colorButtons.value(key)->accessibleName());
	if (!c.isValid())
		return;
	m_editColors.insert(key, c.name());
	updateEditorPreview();
}

void AppearanceTab::updateEditorPreview()
{
	QJsonObject colors;
	for (auto it = m_editColors.cbegin(); it != m_editColors.cend(); ++it)
	{
		colors.insert(it.key(), it.value());
		if (QPushButton* b = m_colorButtons.value(it.key()))
		{
			const QColor c(it.value());
			b->setText(it.value());
			// Schrift schwarz oder weiß, je nach Helligkeit der Farbe.
			b->setStyleSheet(QStringLiteral("background-color:%1; color:%2;").arg(it.value(), c.lightness() < 128 ? QStringLiteral("#ffffff") : QStringLiteral("#000000")));
		}
	}
	m_editPreview->set(m_editName->text(), colors, QString());
}

void AppearanceTab::saveEditor()
{
	QJsonObject colors;
	for (auto it = m_editColors.cbegin(); it != m_editColors.cend(); ++it)
		colors.insert(it.key(), it.value());
	const QJsonObject change { { QStringLiteral("save_palette"),
		                         QJsonObject { { QStringLiteral("name"), m_editName->text() }, { QStringLiteral("dark"), m_editDark->isChecked() }, { QStringLiteral("colors"), colors } } } };
	const QJsonObject r = parse(m_backend->setAppearance(QString::fromUtf8(QJsonDocument(change).toJson(QJsonDocument::Compact))));
	if (r.contains(QStringLiteral("error")))
	{
		emit status(r.value(QStringLiteral("error")).toString());
		return;
	}
	m_editor->hide();
	reload();
	// Die neue Palette gleich verwenden (im automatischen Modus für hell bzw. dunkel).
	tileClicked(r.value(QStringLiteral("saved")).toString(), m_editDark->isChecked());
	emit status(m_backend->text(QStringLiteral("appearance.editor.saved")).arg(m_editName->text()));
}
