#include "mcpserver.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QCryptographicHash>
#include <QtCore/QDir>
#include <QtCore/QFile>
#include <QtCore/QJsonDocument>
#include <QtCore/QPointer>
#include <QtCore/QTimer>
#include <QtNetwork/QLocalServer>
#include <QtNetwork/QLocalSocket>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QPushButton>

McpServer::McpServer(RjBackend* backend, QWidget* window)
  : QObject(window)
  , m_backend(backend)
  , m_window(window)
  , m_server(new QLocalServer(this))
{
	const QString dir = call({ { QStringLiteral("op"), QStringLiteral("data_dir") } }).value(QStringLiteral("path")).toString();
	if (dir.isEmpty())
		return;
	// Je Datenordner ein Kanal (Testinstanzen mit REMOTEJOE_HOME stören sich nicht).
	const QString name = QStringLiteral("remotejoe-mcp-") + QString::fromLatin1(QCryptographicHash::hash(QDir::cleanPath(dir).toUtf8(), QCryptographicHash::Sha256).toHex().left(12));
	QLocalServer::removeServer(name);
	m_server->setSocketOptions(QLocalServer::UserAccessOption);
	if (!m_server->listen(name))
		return;
	QDir().mkpath(dir);
	m_file = QDir(dir).filePath(QStringLiteral("mcp-kanal"));
	QFile f(m_file);
	if (f.open(QIODevice::WriteOnly | QIODevice::Truncate))
		f.write(m_server->fullServerName().toUtf8() + '\n');
	connect(m_server, &QLocalServer::newConnection, this, [this] {
		while (QLocalSocket* s = m_server->nextPendingConnection())
		{
			connect(s, &QLocalSocket::disconnected, s, &QObject::deleteLater);
			connect(s, &QLocalSocket::readyRead, this, [this, s] {
				if (!s->canReadLine() || s->property("busy").toBool())
					return;
				s->setProperty("busy", true);
				const QJsonObject req = QJsonDocument::fromJson(s->readLine()).object();
				handle(s, req);
			});
		}
	});
}

McpServer::~McpServer()
{
	if (!m_file.isEmpty())
		QFile::remove(m_file);
}

QJsonObject McpServer::call(const QJsonObject& cmd) const
{
	return QJsonDocument::fromJson(m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact))).toUtf8()).object();
}

void McpServer::reply(QLocalSocket* s, const QJsonObject& answer)
{
	if (!s || s->state() != QLocalSocket::ConnectedState)
		return;
	s->write(QJsonDocument(answer).toJson(QJsonDocument::Compact) + '\n');
	s->flush();
	s->disconnectFromServer();
}

void McpServer::handle(QLocalSocket* s, const QJsonObject& request)
{
	auto T = [this](const char* k) { return m_backend->text(QString::fromLatin1(k)); };
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("mcp_call") }, { QStringLiteral("tool"), request.value(QStringLiteral("tool")) }, { QStringLiteral("args"), request.value(QStringLiteral("args")) } };
	const QJsonObject first = call(cmd);
	if (!first.value(QStringLiteral("needs_confirm")).toBool())
	{
		reply(s, first);
		return;
	}
	// Aktion: Freigabe durch den Nutzer, höchstens 2 min.
	QPointer<QLocalSocket> sock(s);
	auto* box = new QMessageBox(QMessageBox::Question, T("mcp.confirm.title"), first.value(QStringLiteral("prompt")).toString() + QStringLiteral("\n\n") + T("mcp.confirm.text"), QMessageBox::NoButton, m_window);
	box->setAttribute(Qt::WA_DeleteOnClose);
	box->setWindowModality(Qt::NonModal);
	auto* yes = box->addButton(T("mcp.confirm.yes"), QMessageBox::AcceptRole);
	box->setDefaultButton(box->addButton(T("mcp.confirm.no"), QMessageBox::RejectRole));
	auto* timer = new QTimer(box);
	timer->setSingleShot(true);
	timer->start(120000);
	connect(timer, &QTimer::timeout, box, [box] { box->reject(); });
	connect(box, &QMessageBox::finished, this, [this, box, yes, sock, cmd, first]() mutable {
		if (box->clickedButton() == yes)
		{
			cmd.insert(QStringLiteral("confirmed"), true);
			const QJsonObject r = call(cmd);
			if (r.value(QStringLiteral("freigegeben")).toBool())
				emit actionApproved(r.value(QStringLiteral("action")).toString(), r.value(QStringLiteral("target")).toString());
			reply(sock, r);
		}
		else
		{
			cmd.insert(QStringLiteral("confirmed"), false);
			call({ { QStringLiteral("op"), QStringLiteral("mcp_denied") }, { QStringLiteral("tool"), cmd.value(QStringLiteral("tool")) }, { QStringLiteral("args"), cmd.value(QStringLiteral("args")) } });
			reply(sock, { { QStringLiteral("error"), m_backend->text(QStringLiteral("mcp.denied")) } });
		}
	});
	box->show();
	box->raise();
}
