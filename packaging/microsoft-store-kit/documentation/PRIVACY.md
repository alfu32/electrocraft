# Electrocraft privacy information

Effective date: 10 November 2026. Applies to Electrocraft 0.1.13, published as K3D0.electrocraft by K3D0.

## Local work

Electrocraft edits and simulates circuits on your computer. Schematics, microcontroller programs and custom components are saved to files you choose. Settings, recent-file information, imported component packages and permitted catalog caches are kept in the app's local configuration folder. Core editing and simulation do not require an Electrocraft account or internet access. The current application code does not implement developer-operated advertising, usage analytics or a telemetry upload service.

## Optional supplier catalogs

Online catalogs are disabled by default. You must allow online catalogs in Settings and separately enable/confirm a provider. Searches and part lookups then send the query or manufacturer/vendor part number, relevant region/currency/language settings and authentication data required by that provider to its service. Like other internet requests, these requests expose your network address and request metadata to the recipient.

Providers are JLCPCB/LCSC through the community-operated jlcsearch service at jlcsearch.tscircuit.com, DigiKey at api.digikey.com, Mouser at api.mouser.com, TME at api.tme.eu and Farnell/element14 at api.element14.com. These independent services process requests under their own terms and privacy policies. Links to provider terms are shown in Settings. JLCPCB search does not require a user API key; other integrations require your credentials. No supplier ordering or payment workflow is built into this integration.

Credentials you save are stored in the operating system's credential store (Windows Credential Manager), not in settings or schematic files. Catalog results may be cached locally where provider terms permit. Farnell content is not cached. Sourcing details you choose to record can be included in a saved schematic or exported bill of materials; take care when sharing those files.

## Links, components and automation

Opening a supplier, datasheet or other external link opens another service, normally in your browser; that service's privacy practices apply. Imported JavaScript component code runs in a restricted runtime without file-system or network APIs.

The optional MCP stdio server lets a separately configured client read circuit/simulation information and make edits in the running app. Use clients you trust. A client or AI service may send information elsewhere under its own settings and policies; Electrocraft does not control that external processing. MCP tools cannot enable supplier consent.

## Your controls

Turn off catalog access or individual providers in Settings to stop their requests. Use Forget to remove saved provider credentials and Delete cached data to clear a provider's cache. You control your schematic and export files and can delete them through your file manager. Local configuration and imported packages can be removed from the Electrocraft configuration folder; credential-store entries should be removed using Forget or Windows credential management. Windows and the Microsoft Store may separately process installation, account or diagnostic information under Microsoft's policies.

## Contact

For privacy questions or software issues, contact the project at https://github.com/alfu32/electrocraft/issues. That issue tracker is public: do not post passwords, API keys or private circuit files. This document describes the current implementation; review it when application data handling changes.
