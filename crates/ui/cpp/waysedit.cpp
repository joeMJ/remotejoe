#include "waysedit.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonDocument>
#include <QtCore/QUuid>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QFrame>
#include <QtWidgets/QGridLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QVBoxLayout>

namespace
{
const char* const kProtocols[] = { "web", "ssh", "rdp", "vnc", "psrp" };

QString protocolLabel(const QString& p)
{
	if (p == QLatin1String("web"))
		return QStringLiteral("Web");
	if (p == QLatin1String("psrp"))
		return QStringLiteral("PowerShell");
	return p.toUpper();
}
} // namespace

WaysEditor::WaysEditor(RjBackend* backend, const QJsonObject& data, QWidget* parent)
  : QWidget(parent)
  , m_backend(backend)
  , m_target(data.value(QStringLiteral("id")).toString())
  , m_mainLabel(data.value(QStringLiteral("main_way")).toString())
  , m_logins(data.value(QStringLiteral("way_logins")).toArray())
{
	auto T = [backend](const char* k) { return backend->text(QString::fromLatin1(k)); };
	auto* v = new QVBoxLayout(this);
	v->setContentsMargins(0, 0, 0, 0);
	auto* f = new QFormLayout;
	auto* main = new QLabel(m_mainLabel, this);
	f->addRow(T("ways.main"), main);
	m_openWith = new QComboBox(this);
	f->addRow(T("ways.open"), m_openWith);
	v->addLayout(f);
	auto* intro = new QLabel(T("ways.intro"), this);
	intro->setObjectName(QStringLiteral("rjMuted"));
	intro->setWordWrap(true);
	v->addWidget(intro);
	m_rows = new QVBoxLayout;
	m_rows->setSpacing(8);
	v->addLayout(m_rows);
	auto* buttons = new QHBoxLayout;
	auto* add = new QPushButton(T("ways.add"), this);
	auto* sug = new QPushButton(T("ways.suggest"), this);
	sug->setToolTip(T("ways.suggest.tip"));
	buttons->addWidget(add);
	buttons->addWidget(sug);
	buttons->addStretch(1);
	v->addLayout(buttons);
	m_hint = new QLabel(this);
	m_hint->setObjectName(QStringLiteral("rjMuted"));
	m_hint->setWordWrap(true);
	m_hint->hide();
	v->addWidget(m_hint);
	v->addStretch(1);

	m_loading = true;
	for (const QJsonValue& w : data.value(QStringLiteral("ways")).toArray())
		addRow(w.toObject());
	refreshOpenWith();
	const QString open = data.value(QStringLiteral("open_with")).toString();
	m_openWith->setCurrentIndex(qMax(0, m_openWith->findData(open.isEmpty() ? QStringLiteral("main") : open)));
	m_loading = false;

	connect(add, &QPushButton::clicked, this, [this] {
		QJsonObject w;
		w.insert(QStringLiteral("protocol"), QStringLiteral("web"));
		addRow(w);
		refreshOpenWith();
		emit changed();
	});
	connect(sug, &QPushButton::clicked, this, &WaysEditor::suggest);
	connect(m_openWith, &QComboBox::currentIndexChanged, this, [this] {
		if (!m_loading)
			emit changed();
	});
}

void WaysEditor::addRow(const QJsonObject& w)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	auto* box = new QFrame(this);
	box->setObjectName(QStringLiteral("rjTile"));
	auto* g = new QGridLayout(box);
	g->setContentsMargins(8, 8, 8, 8);
	g->setHorizontalSpacing(8);
	g->setVerticalSpacing(6);
	Row r;
	r.box = box;
	r.id = w.value(QStringLiteral("id")).toString();
	if (r.id.isEmpty())
		r.id = QUuid::createUuid().toString(QUuid::WithoutBraces);
	r.protocol = new QComboBox(box);
	for (const char* p : kProtocols)
		r.protocol->addItem(protocolLabel(QString::fromLatin1(p)), QString::fromLatin1(p));
	r.protocol->setCurrentIndex(qMax(0, r.protocol->findData(w.value(QStringLiteral("protocol")).toString(QStringLiteral("web")))));
	r.name = new QLineEdit(w.value(QStringLiteral("name")).toString(), box);
	r.name->setPlaceholderText(T("ways.name.ph"));
	r.host = new QLineEdit(w.value(QStringLiteral("host")).toString(), box);
	r.host->setPlaceholderText(T("ways.host.ph"));
	r.port = new QLineEdit(w.value(QStringLiteral("port")).isDouble() ? QString::number(w.value(QStringLiteral("port")).toInt()) : QString(), box);
	r.port->setPlaceholderText(T("ways.port.ph"));
	r.port->setMaximumWidth(110);
	r.credential = new QComboBox(box);
	r.credential->addItem(T("ways.cred.target"), QStringLiteral("target"));
	r.credential->addItem(T("ways.cred.own"), QStringLiteral("own"));
	for (const QJsonValue& l : m_logins)
		r.credential->addItem(T("ways.cred.login").arg(l.toObject().value(QStringLiteral("name")).toString()), l.toObject().value(QStringLiteral("id")).toString());
	r.credential->addItem(T("ways.cred.none"), QStringLiteral("none"));
	const QJsonValue cred = w.value(QStringLiteral("credential"));
	r.credential->setCurrentIndex(qMax(0, r.credential->findData(cred.isString() ? cred.toString() : QStringLiteral("target"))));
	// Eigene Zugangsdaten: Benutzer und Kennwort direkt am Weg (eigener KDBX-Eintrag).
	r.ownBox = new QWidget(box);
	auto* oh = new QHBoxLayout(r.ownBox);
	oh->setContentsMargins(0, 0, 0, 0);
	r.user = new QLineEdit(w.value(QStringLiteral("username")).toString(), r.ownBox);
	r.user->setPlaceholderText(T("ways.own.user"));
	r.pass = new QLineEdit(r.ownBox);
	r.pass->setEchoMode(QLineEdit::Password);
	r.pass->setPlaceholderText(cred.toString() == QLatin1String("own") ? T("ways.own.pass.keep") : T("ways.own.pass"));
	oh->addWidget(r.user, 1);
	oh->addWidget(r.pass, 1);
	r.ownBox->setVisible(r.credential->currentData().toString() == QLatin1String("own"));
	auto* remove = new QPushButton(T("ways.remove"), box);
	auto lab = [box](const QString& t) { return new QLabel(t, box); };
	g->addWidget(lab(T("ways.protocol")), 0, 0);
	g->addWidget(r.protocol, 0, 1);
	g->addWidget(lab(T("ways.name")), 0, 2);
	g->addWidget(r.name, 0, 3);
	g->addWidget(remove, 0, 4);
	g->addWidget(lab(T("ways.host")), 1, 0);
	g->addWidget(r.host, 1, 1, 1, 3);
	g->addWidget(r.port, 1, 4);
	g->addWidget(lab(T("ways.cred")), 2, 0);
	g->addWidget(r.credential, 2, 1, 1, 4);
	g->addWidget(r.ownBox, 3, 1, 1, 4);
	g->setColumnStretch(3, 1);
	m_rows->addWidget(box);
	m_list.append(r);

	auto changed = [this] {
		if (m_loading)
			return;
		refreshOpenWith();
		emit this->changed();
	};
	connect(r.protocol, &QComboBox::currentIndexChanged, this, changed);
	connect(r.credential, &QComboBox::currentIndexChanged, this, changed);
	connect(r.credential, &QComboBox::currentIndexChanged, this, [r] { r.ownBox->setVisible(r.credential->currentData().toString() == QLatin1String("own")); });
	connect(r.user, &QLineEdit::textEdited, this, changed);
	connect(r.pass, &QLineEdit::textEdited, this, changed);
	connect(r.name, &QLineEdit::textEdited, this, changed);
	connect(r.host, &QLineEdit::textEdited, this, changed);
	connect(r.port, &QLineEdit::textEdited, this, changed);
	connect(remove, &QPushButton::clicked, this, [this, box] { removeRow(box); });
}

void WaysEditor::removeRow(QWidget* box)
{
	for (int i = 0; i < m_list.size(); ++i)
		if (m_list.at(i).box == box)
		{
			m_list.removeAt(i);
			break;
		}
	box->deleteLater();
	refreshOpenWith();
	emit changed();
}

void WaysEditor::refreshOpenWith()
{
	const QString cur = m_openWith->currentData().toString();
	const bool was = m_loading;
	m_loading = true;
	m_openWith->clear();
	m_openWith->addItem(m_backend->text(QStringLiteral("ways.open.main")).arg(m_mainLabel), QStringLiteral("main"));
	for (const Row& r : std::as_const(m_list))
	{
		const QString label = r.name->text().trimmed().isEmpty() ? r.protocol->currentText() : r.name->text().trimmed();
		m_openWith->addItem(label, r.id);
	}
	m_openWith->setCurrentIndex(qMax(0, m_openWith->findData(cur.isEmpty() ? QStringLiteral("main") : cur)));
	m_loading = was;
}

QJsonArray WaysEditor::ways() const
{
	QJsonArray out;
	for (const Row& r : m_list)
	{
		QJsonObject w;
		w.insert(QStringLiteral("id"), r.id);
		w.insert(QStringLiteral("name"), r.name->text().trimmed());
		w.insert(QStringLiteral("protocol"), r.protocol->currentData().toString());
		w.insert(QStringLiteral("host"), r.host->text().trimmed());
		w.insert(QStringLiteral("port"), r.port->text().trimmed());
		w.insert(QStringLiteral("credential"), r.credential->currentData().toString());
		if (r.credential->currentData().toString() == QLatin1String("own"))
		{
			w.insert(QStringLiteral("username"), r.user->text());
			// Kennwort byte-genau; leer = unverändert.
			if (!r.pass->text().isEmpty())
				w.insert(QStringLiteral("password"), r.pass->text());
		}
		out.append(w);
	}
	return out;
}

QString WaysEditor::openWith() const
{
	return m_openWith->currentData().toString();
}

QString WaysEditor::summary() const
{
	if (m_list.isEmpty())
		return m_backend->text(QStringLiteral("area.ways.none")).arg(m_mainLabel);
	QStringList names;
	for (const Row& r : m_list)
		names << (r.name->text().trimmed().isEmpty() ? r.protocol->currentText() : r.name->text().trimmed());
	return m_backend->text(QStringLiteral("area.ways")).arg(m_mainLabel, names.join(QStringLiteral(", ")));
}

void WaysEditor::suggest()
{
	// Angebotene Dienste aus der Übersicht (Portprüfung): Web auf 4343/443/80, SSH 22, RDP 3389, VNC 5900.
	const QJsonObject r = QJsonDocument::fromJson(
	                          m_backend->edit(QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("op"), QStringLiteral("ways_suggest") }, { QStringLiteral("id"), m_target } }).toJson(QJsonDocument::Compact))).toUtf8())
	                          .object();
	if (r.contains(QStringLiteral("error")))
	{
		m_hint->setText(r.value(QStringLiteral("error")).toString());
		m_hint->show();
		return;
	}
	int added = 0;
	for (const QJsonValue& v : r.value(QStringLiteral("ways")).toArray())
	{
		const QJsonObject w = v.toObject();
		// Schon vorhanden (gleiches Protokoll und gleiche Adresse)? Dann nicht doppelt.
		bool dup = false;
		for (const Row& x : std::as_const(m_list))
			if (x.protocol->currentData().toString() == w.value(QStringLiteral("protocol")).toString() && x.host->text().trimmed() == w.value(QStringLiteral("host")).toString())
				dup = true;
		if (dup)
			continue;
		addRow(w);
		++added;
	}
	refreshOpenWith();
	m_hint->setText(added ? m_backend->text(QStringLiteral("ways.suggest.added")).arg(added) : m_backend->text(QStringLiteral("ways.suggest.none")));
	m_hint->show();
	if (added)
		emit changed();
}

void WaysEditor::wipeSecrets()
{
	for (const Row& r : std::as_const(m_list))
	{
		QString pw = r.pass->text();
		pw.fill(QLatin1Char(' '));
		r.pass->clear();
	}
}
