# S-INFO-444 QUIC - Plan Projet

Objectif: avancer par jalons courts, valider chaque point, garder une trace claire.

## Regles de travail

- Un seul jalon actif a la fois.
- Pas de passage au jalon suivant sans validation.
- Chaque jalon doit avoir: code, test, commande de verification, resultat.

## Jalon 0 - Baseline

But: verifier que la base compile et tourne avant modification.

Checklist:
- [x] Build OK
- [x] Serveur local OK
- [x] Client local OK

Commandes:
- cargo build
- cargo run --bin quiche-server -- --cert apps/src/bin/cert.crt --key apps/src/bin/cert.key
- cargo run --bin quiche-client -- https://test.com --no-verify --connect-to 127.0.0.1:4433

Critere de validation:
- Handshake OK, pas d'erreur protocole.

## Jalon 1 - Transport Parameter seulement

But: negocier enable_server_congestion_resume sans frames.

Fichiers cibles:
- [quiche/src/transport_params.rs](quiche/src/transport_params.rs)
- [quiche/src/lib.rs](quiche/src/lib.rs)
- [apps/src/args.rs](apps/src/args.rs)
- [apps/src/client.rs](apps/src/client.rs)
- [apps/src/bin/quiche-server.rs](apps/src/bin/quiche-server.rs)

Checklist:
- [x] TP ID ajoute (0x3f8e3dbaf71c3293)
- [x] decode TP (zero-length)
- [x] encode TP (zero-length)
- [x] option config ajoutee
- [x] flags CLI client/serveur ajoutes
- [x] test de nego OK

Commandes de verification:
- cargo build
- cargo run --bin quiche-server -- --cert apps/src/bin/cert.crt --key apps/src/bin/cert.key --enable-server-congestion-resume
- cargo run --bin quiche-client -- https://test.com --no-verify --connect-to 127.0.0.1:4433 --enable-server-congestion-resume

Critere de validation:
- Handshake OK avec option activee des deux cotes.

## Jalon 2 - Frames (parse/encode)

- Commit: Add CC_INDICATION & CC_RESUME frames & a verification test
But: ajouter CC_INDICATION et CC_RESUME au format spec.

Fichiers cibles:
- [quiche/src/frame.rs](quiche/src/frame.rs)

Checklist:
- [x] Types frames ajoutes (0x24124758, 0x24124759)
- [x] champs epoch/state/hash geres
- [x] from_bytes OK
- [x] to_bytes OK
- [x] wire_len OK
- [x] tests round-trip OK
`cargo test -p quiche cc_indication`

## Jalon 3 - Regles protocole

Commit : enforce 1-RTT + role checks for CC_INDICATION/CC_RESUME
But: appliquer les contraintes role + type de paquet.

Fichiers cibles:
- [quiche/src/frame.rs](quiche/src/frame.rs)
- [quiche/src/lib.rs](quiche/src/lib.rs)

Checklist:
- [x] frames autorisees uniquement en 1-RTT
- [x] CC_INDICATION: serveur seulement
- [x] CC_RESUME: client seulement
- [x] violations -> PROTOCOL_VIOLATION
- [x] tests negatifs OK

## Jalon 4 - Logique fonctionnelle minimale

Commit : Implement authenticated CC resume flow and apply resumed cwnd across legacy and BBRv2 paths.
But: premier flux bout-en-bout.

Checklist:
- [x] Choix d'implementation (etat CC : cwnd/rtt/rate, `HMAC-SHA256`, expiration : 7j)
- [x] serveur emet CC_INDICATION (changement siginificatif) 
    `cwnd change (+20%), rate (+10%), ou rtt (+10%), ou doublement, avec min 50ms entre deux`
- [x] client stocke/met à jour la derniere valeur
- [x] client envoie CC_RESUME au debut de connexion suivante une seule fois
- [x] serveur accepte ou non de traiter seulement la premiere CC_RESUME

## Jalon 5 - Securite et robustesse

But: couvrir les cas attaques/erreurs demandes.

Checklist:
- [x] hash recalcule cote serveur
- [x] hash invalide ignore silencieusement
- [x] etat expire potentiellement ignore
- [x] tests attaques (forge, replay, multi-resume)


    `Comportement serveur durci pour CC_RESUME dans lib.rs:
        hash invalide → ignoré silencieusement (plus de InvalidPacket)
        état expiré → ignoré silencieusement
        algo mismatch / state invalide / replay après 1er apply → ignorés sans fermer la connexion
        hash toujours recalculé/vérifié côté serveur avant apply_resume_cwnd()`

## Mini checklist rapport final

- [ ] Choix d'implementation (etat CC, hash, expiration)
- [ ] Comment executer client/serveur avec extension
- [ ] Preuves que ca marche (tests + runs)
- [ ] Attaques considerees + defenses
- [ ] Evaluation quantitative (resultats chiffrables)