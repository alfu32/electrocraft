# Electrocraft Privacy Policy

Effective date: 11 November 2026

Publisher: K3D0

Product: Electrocraft, Microsoft Store ID 9MVQ93DQWRWF

This policy describes how Electrocraft handles information when you use its desktop applications, optional component catalogs and MCP server.

## Circuit files and local storage

Electrocraft edits and simulates electronic circuits on your computer. Core editing, simulation and the built-in examples work offline and require no Electrocraft account.

Schematics, microcontroller programs, custom components and exports are saved to locations you choose. Application settings, recent-file information, imported component packages and permitted catalog caches are stored locally. Settings can include file paths, catalog preferences and MCP connection settings. Circuit files and bills of materials can contain manufacturer part numbers and supplier details you choose to record.

Electrocraft does not operate a developer-hosted circuit-storage service. The current application contains no developer-operated advertising, usage analytics or telemetry upload service.

## Optional online component catalogs

Catalog access is off by default. You enable online catalogs in Settings and separately approve each provider. When you search or look up a part, the enabled provider receives the search text or part number, relevant region, currency or language preferences, and the authentication information its API requires. The service also receives your network address and ordinary connection metadata.

The integrations contact these services:

- JLCPCB/LCSC search uses the community-operated jlcsearch service at jlcsearch.tscircuit.com.
- DigiKey uses api.digikey.com.
- Mouser uses api.mouser.com.
- TME uses api.tme.eu.
- Farnell/element14 uses api.element14.com.

These independent services handle requests under their own terms and privacy policies. Provider terms are linked in Settings. JLCPCB search requires no personal API key. Other providers require your own credentials.

Saved supplier credentials are kept in the operating system's credential store, including Windows Credential Manager on Windows. They are not written to schematic files or application settings. Results may be cached locally where the provider permits it. Farnell content is not cached. Supplier information you record in a schematic or export remains in that file until you change or delete it.

## Optional MCP connections

Electrocraft can communicate with a compatible MCP client through standard input/output or through an HTTP or HTTPS server started from Settings. That client can read circuit and simulation information and perform supported actions, including editing schematics and opening, saving or exporting files.

The network listener defaults to the local computer at 127.0.0.1. You choose its bind address, port and transport. A listener bound to a network interface can accept connections from other machines that can reach it and provide its access token. HTTP sends requests and the token without transport encryption. HTTPS encrypts the connection using the certificate and private key you select.

The network access token is generated each time the listener starts and remains in memory while that listener is active. Certificate and private-key file paths are saved in settings. The private-key contents are read from the selected file when starting HTTPS.

When access logging is enabled, Electrocraft retains up to 200 recent entries in memory. Entries contain the client's network address, elapsed time, HTTP method, response status and request duration. TLS handshake failures can also be recorded. The access log does not record bearer tokens, request bodies or tool arguments. You can disable recording or clear the log in Settings. The entries are discarded when the application exits.

An MCP client or AI service may process or transmit information under its own configuration and privacy policy. Use clients you trust and review their handling of circuit information and files. Supplier catalog consent remains controlled through Electrocraft Settings.

## External links and component scripts

Opening a datasheet, supplier page or other external link opens an independent service, usually in your browser. Its own privacy practices apply.

Imported JavaScript component code runs in a restricted runtime without file-system or network APIs.

## Managing your information

You can manage local information and optional connections through these controls:

- Turn off online catalogs or individual providers to stop their requests.
- Use Forget to remove saved provider credentials.
- Use Delete cached data to clear a provider's cache.
- Stop the MCP listener to close its network endpoint.
- Disable or clear the MCP access log in Settings.
- Edit or delete schematic files, exports and imported component packages through the application or your file manager.

Application settings and caches can be removed from the Electrocraft configuration folder. Credential-store entries can be removed through Forget or your operating system's credential manager. Files you share, and information already sent to an independent provider or MCP client, remain subject to the recipient's practices.

Windows and the Microsoft Store may separately process installation, account or diagnostic information under Microsoft's privacy policies.

## Contact and policy updates

For privacy questions, contact the project through its issue tracker at https://github.com/alfu32/electrocraft/issues. Issues are public. Keep passwords, API keys and private circuit files out of issue reports.

This policy describes the current application behavior. Updates to data handling will be reflected in this document and its effective date.
