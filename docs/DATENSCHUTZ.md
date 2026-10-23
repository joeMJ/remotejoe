# Datenschutz und Netzwerkzugriffe

remotejoe verbindet sich nur mit den Zielen, die du einträgst, und – für Updates – mit GitHub:

- **Update-Prüfung:** Bei **Hilfe → Nach Updates suchen …** fragt remotejoe die Veröffentlichungen dieses
  Repositorys bei GitHub ab und lädt auf Wunsch das Paket. Dabei erhält GitHub deine IP-Adresse und übliche technische
  Angaben der Anfrage (z. B. Programmname und Version im User-Agent). Zugangsdaten, Inhalte der Schlüsseldatenbank,
  Verbindungsziele, Hostnamen oder Sitzungsinhalte werden **nicht** übertragen.
- Die Prüfung läuft nur, wenn du sie auslöst. Beim Start prüft remotejoe nur, wenn du „Beim Start prüfen“
  einschaltest (Vorgabe: aus).
- **Proxy:** Im Update-Tab einstellbar: „Automatisch“ nimmt unter Windows die Systemeinstellung (auch mit
  Konfigurationsskript bzw. automatischer Erkennung), sonst `HTTPS_PROXY`/`NO_PROXY`; außerdem „Kein Proxy“ oder
  „Manuell“ (Rechner:Port). Die Verschlüsselung reicht durch den Proxy bis GitHub; eine Anmeldung am Proxy wird nicht
  unterstützt.
- **Hilfe → Fehler melden / Wunsch …** und **Hilfe → Hilfe** öffnen nur Seiten auf GitHub im Standardbrowser; den
  Bericht schickst du dort selbst ab, remotejoe sendet nichts.
- **Objektübersicht:** Beim Einfachklick auf ein Ziel prüft remotejoe die gängigen Ports **dieses Ziels** (TCP-Verbindungsaufbau
  ohne Anmeldung) und fragt bei offenem RDP mit NLA den Rechnernamen ab (ohne Zugangsdaten). Ping und Traceroute laufen
  nur auf Knopfdruck, ebenso „Inventar abfragen“ und das Masseninventar mit den Zugangsdaten des Ziels (nur bei
  Zielen, deren Inventar erlaubt ist). Es geht nichts an Dritte.
- **Aruba-Werkzeuge** (nur wenn eingeschaltet): „Einrichten“ lädt die Aruba Instant Python Tools von GitHub und die
  Python-Bibliotheken über pip, unter Windows bei Bedarf Python von python.org. Abfragen und Werkzeuge verbinden sich
  nur mit den Aruba-Controllern bzw. APs; beim Firmware-Update lädt der AP die Firmware selbst von der angegebenen URL.
- Es gibt keine Telemetrie und keine Nutzungsstatistik.
