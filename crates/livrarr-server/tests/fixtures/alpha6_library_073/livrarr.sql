PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
INSERT INTO _sqlx_migrations VALUES(1,'initial schema','2026-09-28 06:13:10',1,X'5d6f3ef5ecbe594e084b2d9b6689392c342451a0c287b36c21f1afaced52d5899402827393554a2c12a47d2a39bc4ed3',1909881);
INSERT INTO _sqlx_migrations VALUES(2,'add indexers','2026-09-28 06:13:10',1,X'7b67c9fe7212481ea79077acec8d71772c173a3d16835cb6006b33d5fd905f0e94037a17a9f4815b76e7d8d4f669efb8',244247);
INSERT INTO _sqlx_migrations VALUES(3,'add author bibliography','2026-09-28 06:13:10',1,X'ffc1401a3f83da0810095e9a62a4fcd01d4854b7bc8c81811fac6e2dc1fe2eb339f992537d5916c30c77f8e8ae3eee6d',208022);
INSERT INTO _sqlx_migrations VALUES(4,'remove userid from paths','2026-09-28 06:13:10',1,X'dac7529b20122d5333e4dc1c83fb2a6f76a64a4b5b2f5d9e6b06b844d98c3e616efff047fae0fbd82cb04bd5c362f3ad',118678);
INSERT INTO _sqlx_migrations VALUES(5,'add preferred formats','2026-09-28 06:13:10',1,X'aabb28e93316cd26ff0f38e687df998ed6cf21f866ee47087b896563979842ed2faeb486a52baa2a0ab5f6956e7174de',767534);
INSERT INTO _sqlx_migrations VALUES(6,'add usenet support','2026-09-28 06:13:10',1,X'b6fef77d472814a543ee64bdcbeaaf69b6c9f3c64ec8511a33a8811ac37e5c2246b38e904cf53bbd7e27b3f4a4a7fa4c',1145092);
INSERT INTO _sqlx_migrations VALUES(7,'add indexer protocol','2026-09-28 06:13:10',1,X'0ca55bff6375608847a53909700b7e18d9beec0c4f01c7bd7907fc45850c1c9f71cc5910a76abecd810ea31291228395',472490);
INSERT INTO _sqlx_migrations VALUES(8,'add grab media type','2026-09-28 06:13:10',1,X'2860fc7fcb60be7e21831aa6c65c45f7f90f1015adc48c13acf41b2722199e41f7ab2f24c8fcef7faf1689e69649fb99',449611);
INSERT INTO _sqlx_migrations VALUES(9,'add metadata enable flags','2026-09-28 06:13:10',1,X'f93fa82d3e1d1b9fc04451c4c1c7f35d4c511fbbb0675c946342e58ea1ce258069aaa424f83cf830fe3a5d428ecbe637',749416);
INSERT INTO _sqlx_migrations VALUES(10,'add livrarr meta','2026-09-28 06:13:10',1,X'43417aac9b15af9355da3fad9d4f7ab10c3d78f8386d2109c4fe354f77d53befa0525d868f1ec2441c156b9515424691',223690);
INSERT INTO _sqlx_migrations VALUES(11,'add email config','2026-09-28 06:13:10',1,X'40c31f0d1868d8084567fc403543c0c847b124122ca818946282d15226777f25ea92a824bc4cd2d46a7555b1393757d7',240268);
INSERT INTO _sqlx_migrations VALUES(12,'add metadata source','2026-09-28 06:13:10',1,X'41aa9f0903764ded996d546035c73a852aaf6c9a7ec85227baa4d3ec8a0d023aabef41d6044622c67251eb6ebfa672c1',508083);
INSERT INTO _sqlx_migrations VALUES(13,'add detail url','2026-09-28 06:13:10',1,X'49de27bea79282bb7ee06a223aa445305e337bace7124f7269319eb90097d6faba4384ac635434cc25a8efe710f692be',511684);
INSERT INTO _sqlx_migrations VALUES(14,'add grab content path','2026-09-28 06:13:10',1,X'874f71e140a15b4e55d9239a611e25fdb041b0ec3377f433ff68513e0ad87e3f3e3e01ea6569d3ce69fd0d620099bc5d',714307);
INSERT INTO _sqlx_migrations VALUES(15,'rss sync','2026-09-28 06:13:10',1,X'ae83ba3803edebfe92d1afb4de5acbe6d40cea92d2d4241295fc359b90137b54d6966da7ac587f76c4d253f9dd59d7f6',2756135);
INSERT INTO _sqlx_migrations VALUES(16,'list imports','2026-09-28 06:13:10',1,X'e2692abc11fa4bccf8272cb997191e433a4784e1d3c2a0a0cb1083b5cb2ece887e7cf9d20fa222408c319959fa95dd07',2818034);
INSERT INTO _sqlx_migrations VALUES(17,'readarr import','2026-09-28 06:13:10',1,X'586ddd25f3439f553079ee26ff83393f882263b066b8d5f0a3873ec2021f5e828d9cff9f3b1a326f7439195594265475',1427664);
INSERT INTO _sqlx_migrations VALUES(18,'playback progress','2026-09-28 06:13:10',1,X'3b1001a6b38271a3c4560dbb52b0623bacadb1b9011c9e3717c16dd2bee73a36ef6d5ec758282a6a90c6555bf9ee4054',265207);
INSERT INTO _sqlx_migrations VALUES(19,'fix playback progress cascade','2026-09-28 06:13:10',1,X'0fbf1f2f8ae2d5831c90133ee93b55ad1f5047a60074d79c9e47235785569889604be48c8b669fe58a24cf24da63ecd2',1542034);
INSERT INTO _sqlx_migrations VALUES(20,'list import previews','2026-09-28 06:13:10',1,X'135081ebc9f575949c4ac9d67e5aa41863cd6914c5f48dd9e0810feb986e908e6d02326935ad1e4b7d75761ab47de93d',309976);
INSERT INTO _sqlx_migrations VALUES(21,'add library item imported at','2026-09-28 06:13:10',1,X'3602d3b198009edf69cb273cf89af9d9c8a703178c4600af33d8e6581c87bd106fd26c1a058ebc084e6792334ff1813b',103667);
INSERT INTO _sqlx_migrations VALUES(22,'backfill imported at','2026-09-28 06:13:10',1,X'8309a7e21541c69cd3f81bf302f15c5bb1a9d4c0331d76e848d3348bfb5ae52ac77fcc6fa88ff0ab4be4a6244df78399',96339);
INSERT INTO _sqlx_migrations VALUES(23,'series monitoring','2026-09-28 06:13:10',1,X'332d650666b594c6598c14ea0ad74cfa85b97a62b503166c532c328e2e14969ce9e6143f902b04eed320c72ca02c40c1',917452);
INSERT INTO _sqlx_migrations VALUES(24,'import retry','2026-09-28 06:13:10',1,X'74e60e1ef89380c1cdc3739f282beccf3d02c0cd6692ce569b74e9a920b231199834c9f0f9857b96b2ac77fad756ef3c',1054155);
INSERT INTO _sqlx_migrations VALUES(25,'fix path corruption','2026-09-28 06:13:10',1,X'9a12d16f2305d0781eb5695ee197ce1ebc4d0152fbfbb23dc219b0a9db56707927eb538b066b9c5b73d2495036aaaf29',134315);
INSERT INTO _sqlx_migrations VALUES(26,'cascade user fk','2026-09-28 06:13:10',1,X'a20dbc46aa865587627d054d6ca14776991f444145ecdfd4b5eca302de2eba66bf33896fb46df5e814a0c164044c441c',5506428);
INSERT INTO _sqlx_migrations VALUES(27,'finalize schema version','2026-09-28 06:13:10',1,X'965aa665abf6a0cf696d04342b120749b28d1d4ba2a08bf7e8e3fa1019e9ffbd79fde7364354097d44e2c31efbcd2907',167696);
INSERT INTO _sqlx_migrations VALUES(28,'add work metadata provenance','2026-09-28 06:13:10',1,X'417fa171929c141758c7beb8dd8247486535e7d4efbd560cf989e6222238382b55e3614d7d7ccc20b2b96477ce243335',273868);
INSERT INTO _sqlx_migrations VALUES(29,'add provider retry state','2026-09-28 06:13:10',1,X'259c6b6faeb3a05abde8d4900f3ffed531ca5119491f62a7ce90469b075bccc1ef7313530826b305dcc3fda177e13321',232337);
INSERT INTO _sqlx_migrations VALUES(30,'add merge generation','2026-09-28 06:13:10',1,X'de0853bd99b95fb379563313b4897985b47a5b64b3af4067e208b783a4e657a841cad321a8331977e76f5ca6e006ae18',681146);
INSERT INTO _sqlx_migrations VALUES(31,'session cleanup index','2026-09-28 06:13:10',1,X'0d65e5a6a50301adab0bdc3ccec13f62484d9e71b6c4e1e80c268743636c128f413190ead285d7e163558bee574eae24',167477);
INSERT INTO _sqlx_migrations VALUES(32,'notification dedup index','2026-09-28 06:13:10',1,X'90e28399911d3150f92d35b6103c9cb34905d8772dc454e12e86782e7b48ef68ea51ba72e14d1643b1b85d1e446cc2b6',132506);
INSERT INTO _sqlx_migrations VALUES(33,'add raw entries','2026-09-28 06:13:10',1,X'14f6e78042ddf01e3b253d899530cadc1eb2acfc8ab0f8c14f587725490b7caaf147d6c4a87dcfabd684a868ff1a54fa',1146703);
INSERT INTO _sqlx_migrations VALUES(34,'bump schema version','2026-09-28 06:13:10',1,X'7f3835e9688306d9401b4d078696fb1164aa877d0be8b8c5eb4d19d7adf9479c923b67d485e41e30ebf35e450b884e96',101020);
INSERT INTO _sqlx_migrations VALUES(35,'enrichment status rename','2026-09-28 06:13:10',1,X'312091e59179b28e1cb97e39eec259fd3a4843319aba51064a91f0fb277b4768a40aab61bf59a6979735bb72e1cbefd0',124085);
INSERT INTO _sqlx_migrations VALUES(36,'library item tag tracking','2026-09-28 06:13:10',1,X'ac39ec693651e4379776752da11422ba12c48e4a94124ba901503ca337bf196addd95193db6b360132eb729b6133b33d',1115983);
INSERT INTO _sqlx_migrations VALUES(38,'normalized identity','2026-09-28 06:13:10',1,X'4945c73ee139a72c07c1e0a4410f4a6ea147f2a367dce92eb2ebf3fd3308027e7b4bc261c7d338787b3e847706734c07',1048676);
INSERT INTO _sqlx_migrations VALUES(39,'work identity anchors','2026-09-28 06:13:10',1,X'2bf006bffa7564e78329ae0463856092f150db20b3f65fbbd612353849e8746f668d03e4cb254eafb9bf23a20bf6b8d7',334438);
INSERT INTO _sqlx_migrations VALUES(40,'work identity conflicts','2026-09-28 06:13:10',1,X'8d8cdf03d351806ebb90b859aa9f3ada34a2adba9796156443f12f3fede061bfd1444c2931c5eb03ab93135dfcc45437',306691);
INSERT INTO _sqlx_migrations VALUES(41,'anchor user uniqueness','2026-09-28 06:13:10',1,X'18df20067186cf3d6831fe09c816977fe6e30dd48d69ff3af9591a1d777c3267f6721377691786b3d27bac185edd9546',150156);
INSERT INTO _sqlx_migrations VALUES(42,'fix anchor user scope','2026-09-28 06:13:10',1,X'1d574fc4fb87733c04804ec66b0ee29bb022ec82f2edcae698a5483a249e00ab7798ebdbd3494f5e2c54c5b16ee132e2',145772);
INSERT INTO _sqlx_migrations VALUES(43,'backfill ol anchors','2026-09-28 06:13:10',1,X'7df5f5fe05a3bb889c3dc6be17a58652e14b36809a06295ce39e1134293312c661136fbbcc3b1ba87f81c30ac6f84313',129770);
INSERT INTO _sqlx_migrations VALUES(44,'anchor per user uniqueness','2026-09-28 06:13:10',1,X'dcc8a840569cd8bddb33861d52c575acfa102b99005cff8067457fe56cc70c39223852577c6200a76c036abff7cf7024',699294);
INSERT INTO _sqlx_migrations VALUES(45,'add cover trust and audiobook','2026-09-28 06:13:10',1,X'22c9918492804d21372ead30a18fca20b5185c5730a74c35ca20080a59a75885f541eb035a9857c8658ded42a3dc4f02',4768283);
INSERT INTO _sqlx_migrations VALUES(46,'add download dir','2026-09-28 06:13:10',1,X'f94e79eee92ed4232d7237884fe4bbe9de71ce0509d8a9dba7dde5a0d5a949d011aa80f79cd20537c737d20f87755209',733295);
INSERT INTO _sqlx_migrations VALUES(47,'add google books api key','2026-09-28 06:13:10',1,X'7fcfa690b6e7c44a78553d050507878b08dac77dd9a2b74684d8f087764e4b5e828931db0425aa17f6f70f3764391923',707521);
INSERT INTO _sqlx_migrations VALUES(48,'add audiobook chapters','2026-09-28 06:13:10',1,X'3527ecc0fdfcd12972e8ff63d9b5fe93067b4784df87edfcd35e8b587012b27277a1491fe744bee9f8563060e4a7280b',288487);
INSERT INTO _sqlx_migrations VALUES(49,'add bookmarks','2026-09-28 06:13:10',1,X'66fb46243c25be5abb1d9cc1a2e37646ba1e12a036d73ee505244ea4b297ef7d28aca616cdb7fcca2222c1b3e33446aa',310275);
INSERT INTO _sqlx_migrations VALUES(50,'add progress finished at and item duration','2026-09-28 06:13:10',1,X'21714506ecee6a9558596cd3dfe06f9bc17ea70b3f4469da35a83147d4d8d586aec842a801c416a075d9b836e29aff3d',1921921);
INSERT INTO _sqlx_migrations VALUES(51,'migrate gemini preview model','2026-09-28 06:13:10',1,X'042f6ccfa612075ff43d756b2ba1231b213b6e9c7aa37c2186a02566a9467316eaea11dbb80c9433e05403b0364365d6',104593);
INSERT INTO _sqlx_migrations VALUES(52,'federate identity conflicts','2026-09-28 06:13:10',1,X'17d0092b698d5075451b0ca0c6b546fae52de40f8b796ce0ff3c148db05231900e05320528d8825b296ab8d4c442e717',2184385);
INSERT INTO _sqlx_migrations VALUES(53,'list preview goodreads book id','2026-09-28 06:13:10',1,X'720251f21304bec054b6e595249a88d6297138d2d24aec249d84baa98a7b68d85a1d18bebfcdffce753171bd18bf7efb',777882);
INSERT INTO _sqlx_migrations VALUES(54,'add identity status','2026-09-28 06:13:10',1,X'58c7e547fa9949503293826d091b0a63cdf4e2e064c8439a52aac08e3064244dd0df5ab560cf6fc441a8739c6ae265f3',840799);
INSERT INTO _sqlx_migrations VALUES(55,'drop enrichment identity variants','2026-09-28 06:13:10',1,X'eafcfea336e4b1869bb0b5d05a3576d38ec833d6ccfb5c0a1ed7b7252661f0304698334c69125a7237cc53e25419fb30',178599);
INSERT INTO _sqlx_migrations VALUES(56,'metadata cache','2026-09-28 06:13:10',1,X'a3366ca6a4a810f1295c0ac4ca43d60e7ef0bc4f5f4e88b4ffcca7303288306cd5c702d53441c84da3af41611ce8abe3',209015);
INSERT INTO _sqlx_migrations VALUES(57,'provider policy','2026-09-28 06:13:10',1,X'cf291b428d62e86e99516b7fb3f49f3391beef0243114135c0adf278a89c0c889914ecbf40f9bba6b1a89ca5b9334f5c',230491);
INSERT INTO _sqlx_migrations VALUES(58,'cross format','2026-09-28 06:13:10',1,X'084f1d383dd3fe4fa725b9f7cd0f28f41387d4ff2d79c32eefcdd6acb52ea93ca95892d9a400a8e5215a5b99650ca073',378987);
INSERT INTO _sqlx_migrations VALUES(59,'provider call records','2026-09-28 06:13:10',1,X'08d4adb48cf6bb9613bdf6cc1812294ae342f5eddc0f8cbe713e36899c5da985445143d1a47f204b198153033a535c7b',302746);
INSERT INTO _sqlx_migrations VALUES(60,'work field dissents','2026-09-28 06:13:10',1,X'7ae763990133dc96bd89b3384405661ae176e03aae0ada2f188f38833d4981220eb5476ebcdc72d77ffaa66746a8d249',247606);
INSERT INTO _sqlx_migrations VALUES(61,'drop metadata source','2026-09-28 06:13:10',1,X'dfb78949e88412087260df9f300f19e579f532ea9f9e323c36b544d4b477e071fb478bb619b553e69ffc9f7bba271365',3282963);
INSERT INTO _sqlx_migrations VALUES(62,'series roster','2026-09-28 06:13:10',1,X'12e5f5d96811062e73229f201d4ff1841889b1487d1c3ccb663a3ae94e1d3e36c982277a0831dce56e2913f61eea45b9',250555);
INSERT INTO _sqlx_migrations VALUES(63,'monitor language','2026-09-28 06:13:10',1,X'98852c09039b3b1cd41e927c7ab1083c404d36cd06603fa5efd5856d114f81db99b289dda225c84c77f4e9808f489209',1581836);
INSERT INTO _sqlx_migrations VALUES(64,'work anchor dead ends','2026-09-28 06:13:10',1,X'18545357b38947ce46f8bdd230945ed61cb63df47f79db28914065970db96e9cef27a37f8f9f3c23bf0158952f9871b9',255107);
INSERT INTO _sqlx_migrations VALUES(65,'works next convergence at','2026-09-28 06:13:10',1,X'91ee6e49fe8d39d17e1891dbb51441174bd862bb1bf10cb3c80693ff1d6d5eaa23a088bbb3a5e5c106965ca6af8b1109',921689);
INSERT INTO _sqlx_migrations VALUES(66,'drop metadata cache','2026-09-28 06:13:10',1,X'8286fc500980202043b3679437ef9a8256a8ecde509f671b5b518c1a69dae4a619462ff9496592cffbe9b50c92cf00a9',178903);
INSERT INTO _sqlx_migrations VALUES(67,'user default language','2026-09-28 06:13:10',1,X'9cbd4a054240f8349671e374f1bb554728c4ac09abbad5c13cffa633cdf67c60dc27d6df6957ee6c9cdc3762801e3748',862041);
INSERT INTO _sqlx_migrations VALUES(68,'work identity review candidates','2026-09-28 06:13:10',1,X'4348f22e52c667d163a0205daf4ccd25e3a49aa7b1a95d650211ec4a46f48730c4c54d96633101f7bdb7c0156d7f318e',253856);
INSERT INTO _sqlx_migrations VALUES(69,'identity key generation','2026-09-28 06:13:10',1,X'c9576e36382d0cf3be5c2dd4836bb847a06e095986a3dc5ec80a35da3cbf880a77d2d19d88d179a742e2c559b4012079',104233);
INSERT INTO _sqlx_migrations VALUES(70,'drop enrichment retry count','2026-09-28 06:13:10',1,X'ebe9a036683023e1a85b0907120f57dbae27cbf0a20611b4d2ca90020e337ae9f20400b3be4a6792e9e792cd9929a113',3001961);
INSERT INTO _sqlx_migrations VALUES(71,'rss grab failure limit','2026-09-28 06:13:10',1,X'413761d728442324ed1419525bc89f805b6a43fa9543b560180f0138dac47bc2916ab5bf45fdcacf799d544a3d621490',832927);
INSERT INTO _sqlx_migrations VALUES(72,'provider response cache','2026-09-28 06:13:10',1,X'ea03441adf69350986184aced806c15cdedfad348fbc8a46c75ca9f95bbd68c4f289e7807da6e26cff5227dc20e5fd94',320431);
INSERT INTO _sqlx_migrations VALUES(73,'drop suppression columns','2026-09-28 06:13:10',1,X'3b8c9e31d7350453c9b3bc3050948fa9a7faaee4647c3559d92c13de054375463d4e74f8ea8bcc0d515ed45d02c03d56',6209344);
CREATE TABLE users (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL,
    password_hash   TEXT NOT NULL,
    role            TEXT NOT NULL CHECK(role IN ('admin', 'user')),
    api_key_hash    TEXT NOT NULL UNIQUE,
    setup_pending   INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
INSERT INTO users VALUES(1,'alpha6-admin','$argon2id$v=19$m=19456,t=2,p=1$J6J8vUWMR7bYlwiMGg5NcA$+mJlIXxVjhk/0vWBW/tJeV6QzShzh86ccpPNzxUhaCI','admin','54772cc442f2014c3be44be9fe8e0b64e8ca952dac1a17f378930038c72d1c8d',0,'2026-09-28T06:13:10Z','2026-09-28T06:13:11.292220816+00:00');
INSERT INTO users VALUES(2,'second-reader','$argon2id$v=19$m=19456,t=2,p=1$J6J8vUWMR7bYlwiMGg5NcA$+mJlIXxVjhk/0vWBW/tJeV6QzShzh86ccpPNzxUhaCI','user','a62f66cba6de5e6d68c713fb0a4b95866646a82d0a965a884f3c225a47b846f3',0,'2026-06-01T12:00:00+00:00','2026-06-01T12:00:00+00:00');
CREATE TABLE sessions (
    token_hash      TEXT PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    persistent      INTEGER NOT NULL,
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL
);
CREATE TABLE authors (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    sort_name           TEXT,
    ol_key              TEXT,
    monitored           INTEGER NOT NULL DEFAULT 0,
    monitor_new_items   INTEGER NOT NULL DEFAULT 0,
    monitor_since       TEXT,
    added_at            TEXT NOT NULL
, gr_key TEXT, hc_key TEXT, import_id TEXT REFERENCES imports(id), monitor_language TEXT);
INSERT INTO authors VALUES(1,1,'Ursula K. Le Guin','Ursula K. Le Guin',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(2,1,'Frank Herbert','Frank Herbert',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(3,1,'Ann Leckie','Ann Leckie',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(4,1,'Octavia E. Butler','Octavia E. Butler',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(5,1,'Iain M. Banks','Iain M. Banks',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(6,1,'N. K. Jemisin','N. K. Jemisin',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(7,1,'Becky Chambers','Becky Chambers',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(8,1,'James S. A. Corey','James S. A. Corey',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(9,1,'Dan Simmons','Dan Simmons',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(10,1,'Emily St. John Mandel','Emily St. John Mandel',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(11,1,'William Gibson','William Gibson',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(12,1,'Adrian Tchaikovsky','Adrian Tchaikovsky',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES(20,2,'Frank Herbert','Frank Herbert',NULL,0,0,NULL,'2026-06-01T12:00:00+00:00',NULL,NULL,NULL,NULL);
CREATE TABLE works (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id                 INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title                   TEXT NOT NULL,
    sort_title              TEXT,
    subtitle                TEXT,
    original_title          TEXT,
    author_name             TEXT NOT NULL,
    author_id               INTEGER REFERENCES authors(id) ON DELETE SET NULL,
    description             TEXT,
    year                    INTEGER,
    series_name             TEXT,
    series_position         REAL,
    genres                  TEXT,       -- JSON array
    language                TEXT,
    page_count              INTEGER,
    duration_seconds        INTEGER,
    publisher               TEXT,
    publish_date            TEXT,
    ol_key                  TEXT,
    hc_key            TEXT,
    isbn_13                 TEXT,
    asin                    TEXT,
    narrator                TEXT,       -- JSON array
    narration_type          TEXT,
    abridged                INTEGER DEFAULT 0,
    rating                  REAL,
    rating_count            INTEGER,
    enrichment_status       TEXT NOT NULL DEFAULT 'pending',
    enriched_at             TEXT,
    enrichment_source       TEXT,
    cover_url               TEXT,
    cover_manual            INTEGER NOT NULL DEFAULT 0,
    monitor_ebook               INTEGER NOT NULL DEFAULT 1,
    added_at                TEXT NOT NULL,
    detail_url TEXT, monitor_audiobook BOOLEAN NOT NULL DEFAULT 0, gr_key TEXT, import_id TEXT REFERENCES imports(id), series_id INTEGER REFERENCES series(id) ON DELETE SET NULL, merge_generation INTEGER NOT NULL DEFAULT 0, normalized_title TEXT NOT NULL DEFAULT '__UNMIGRATED__', normalized_author TEXT NOT NULL DEFAULT '__UNMIGRATED__', cover_source TEXT, cover_trust TEXT DEFAULT 'unvalidated', cover_width INTEGER DEFAULT 0, cover_height INTEGER DEFAULT 0, audiobook_cover_url TEXT, audiobook_cover_source TEXT, audiobook_cover_trust TEXT DEFAULT 'unvalidated', audiobook_cover_width INTEGER DEFAULT 0, audiobook_cover_height INTEGER DEFAULT 0, identity_status TEXT NOT NULL DEFAULT 'pending', next_convergence_at TEXT);
INSERT INTO works VALUES(1,1,'A Wizard of Earthsea',NULL,'The First Book of Earthsea',NULL,'Ursula K. Le Guin',1,NULL,NULL,'Earthsea Cycle',1.5,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900001W','upgrade-fixture-900001','9780009000010','B0UPG00001',NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover','https://covers.openlibrary.org/b/id/900001-L.jpg',1,1,'2026-06-01T12:00:00+00:00',NULL,1,'900001',NULL,1,0,'wizard of earthsea','ursula k le guin','openlibrary','user',500,500,'https://m.media-amazon.com/images/I/900001.jpg','audible','validated',500,500,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(2,1,'Dune',NULL,NULL,NULL,'Frank Herbert',2,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900002W','upgrade-fixture-900002','9780009000027','B0UPG00002',NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,0,'2026-06-01T12:00:00+00:00',NULL,1,'900002',NULL,NULL,0,'dune','frank herbert',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(3,1,'Ancillary Justice',NULL,NULL,NULL,'Ann Leckie',3,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900003W','upgrade-fixture-900003','9780009000034','B0UPG00003',NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,'900003',NULL,NULL,0,'ancillary justice','ann leckie',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(4,1,'Kindred',NULL,NULL,NULL,'Octavia E. Butler',4,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900004W','upgrade-fixture-900004','9780009000041','B0UPG00004',NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,'900004',NULL,NULL,0,'kindred','octavia e butler',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'pending','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(5,1,'The Player of Games',NULL,NULL,NULL,'Iain M. Banks',5,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'player of games','iain m banks',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'pending','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(6,1,'The Fifth Season',NULL,NULL,NULL,'N. K. Jemisin',6,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900006W',NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'fifth season','n k jemisin',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'needs_review','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(7,1,'The Long Way to a Small, Angry Planet',NULL,NULL,NULL,'Becky Chambers',7,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,'900007',NULL,NULL,0,'long way to a small angry planet','becky chambers',NULL,'unvalidated',0,0,'https://images.gr-assets.com/books/900007l.jpg','goodreads','user',500,500,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(8,1,'Record of a Spaceborn Few',NULL,NULL,NULL,'Becky Chambers',7,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,'900008',NULL,NULL,0,'record of a spaceborn few','becky chambers',NULL,'unvalidated',0,0,'https://images.gr-assets.com/books/900008l.jpg','goodreads','validated',500,500,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(9,1,'Children of Time',NULL,NULL,NULL,'Adrian Tchaikovsky',12,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900009W','upgrade-fixture-900009','9780009000096','B0UPG00009',NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,'900009',NULL,NULL,0,'children of time','adrian tchaikovsky',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(10,1,'Leviathan Wakes',NULL,NULL,NULL,'James S. A. Corey',8,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'leviathan wakes','james s a corey',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'pending','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(11,1,'Leviathan Wakes',NULL,NULL,NULL,'Daniel Abraham',8,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'leviathan wakes','daniel abraham',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'pending','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(12,1,'Leviathan Wakes',NULL,NULL,NULL,'Ty Franck',8,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'leviathan wakes','ty franck',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'pending','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(13,1,'Hyperion',NULL,NULL,NULL,'Dan Simmons',9,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,'900013',NULL,NULL,0,'hyperion','dan simmons',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(14,1,'The Fall of Hyperion',NULL,NULL,NULL,'Dan Simmons',9,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,'900013',NULL,NULL,0,'fall of hyperion','dan simmons',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(15,1,'Station Eleven',NULL,NULL,NULL,'Emily St. John Mandel',10,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,' /works/OL900015W',NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'station eleven','emily st john mandel',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(16,1,'Sea of Tranquility',NULL,NULL,NULL,'Emily St. John Mandel',10,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,replace('	/works/OL900015W\n','\n',char(10)),NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'sea of tranquility','emily st john mandel',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(17,1,'The Glass Hotel',NULL,NULL,NULL,'Emily St. John Mandel',10,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'  	',NULL,NULL,'　',NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'glass hotel','emily st john mandel',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'pending','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(18,1,'Neuromancer',NULL,NULL,NULL,'William Gibson',11,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900018W',NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'neuromancer','william gibson',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'conflict','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(19,1,'Count Zero',NULL,NULL,NULL,'William Gibson',11,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900019W',NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'count zero','william gibson',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
INSERT INTO works VALUES(20,2,'Dune',NULL,NULL,NULL,'Frank Herbert',20,NULL,NULL,NULL,NULL,NULL,'en',NULL,NULL,NULL,NULL,'/works/OL900002W',NULL,NULL,NULL,NULL,NULL,0,NULL,NULL,'enriched','2026-06-01T12:00:00+00:00','hardcover',NULL,0,1,'2026-06-01T12:00:00+00:00',NULL,0,NULL,NULL,NULL,0,'dune','frank herbert',NULL,'unvalidated',0,0,NULL,NULL,'unvalidated',0,0,'confirmed','2099-01-01T00:00:00+00:00');
CREATE TABLE external_ids (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    work_id     INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    id_type     TEXT NOT NULL,
    id_value    TEXT NOT NULL,
    UNIQUE(work_id, id_type, id_value)
);
CREATE TABLE root_folders (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT NOT NULL UNIQUE,
    media_type  TEXT NOT NULL CHECK(media_type IN ('ebook', 'audiobook'))
);
CREATE TABLE library_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id         INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    root_folder_id  INTEGER NOT NULL REFERENCES root_folders(id) ON DELETE RESTRICT,
    path            TEXT NOT NULL,
    media_type      TEXT NOT NULL CHECK(media_type IN ('ebook', 'audiobook')),
    file_size       INTEGER NOT NULL,
    imported_at     TEXT NOT NULL, import_id TEXT REFERENCES imports(id), tag_status TEXT NOT NULL DEFAULT 'pending', tagged_at_generation INTEGER NOT NULL DEFAULT 0, duration_seconds REAL, chapter_scan_status TEXT,
    UNIQUE(user_id, root_folder_id, path)
);
CREATE TABLE naming_config (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    author_folder_format    TEXT NOT NULL DEFAULT '{Author Name}',
    book_folder_format      TEXT NOT NULL DEFAULT '{Book Title}',
    rename_files            INTEGER NOT NULL DEFAULT 0,
    replace_illegal_chars   INTEGER NOT NULL DEFAULT 1
);
INSERT INTO naming_config VALUES(1,'{Author Name}','{Book Title}',0,1);
CREATE TABLE media_management_config (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    cwa_ingest_path TEXT
, preferred_ebook_formats TEXT NOT NULL DEFAULT '["epub"]', preferred_audiobook_formats TEXT NOT NULL DEFAULT '["m4b"]');
INSERT INTO media_management_config VALUES(1,NULL,'["epub"]','["m4b"]');
CREATE TABLE prowlarr_config (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    url     TEXT,
    api_key TEXT,
    enabled INTEGER NOT NULL DEFAULT 0
);
INSERT INTO prowlarr_config VALUES(1,NULL,NULL,0);
CREATE TABLE metadata_config (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    hardcover_api_token TEXT,
    llm_provider        TEXT,
    llm_endpoint        TEXT,
    llm_api_key         TEXT,
    llm_model           TEXT,
    audnexus_url        TEXT NOT NULL DEFAULT 'https://api.audnex.us',
    languages           TEXT NOT NULL DEFAULT '["en"]'
, hardcover_enabled INTEGER NOT NULL DEFAULT 1, llm_enabled INTEGER NOT NULL DEFAULT 1, google_books_api_key TEXT DEFAULT NULL, default_language TEXT NOT NULL DEFAULT 'en');
INSERT INTO metadata_config VALUES(1,NULL,NULL,NULL,NULL,NULL,'https://api.audnex.us','["en"]',1,1,NULL,'en');
CREATE TABLE download_clients (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT NOT NULL,
    implementation      TEXT NOT NULL DEFAULT 'qBittorrent',
    host                TEXT NOT NULL,
    port                INTEGER NOT NULL DEFAULT 8080,
    use_ssl             INTEGER NOT NULL DEFAULT 0,
    skip_ssl_validation INTEGER NOT NULL DEFAULT 0,
    url_base            TEXT,
    username            TEXT,
    password            TEXT,
    category            TEXT NOT NULL DEFAULT 'livrarr',
    enabled             INTEGER NOT NULL DEFAULT 1
, client_type TEXT NOT NULL DEFAULT 'qbittorrent', api_key TEXT, is_default_for_protocol BOOLEAN NOT NULL DEFAULT false, download_dir TEXT);
CREATE TABLE remote_path_mappings (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    host        TEXT NOT NULL,
    remote_path TEXT NOT NULL,
    local_path  TEXT NOT NULL
);
CREATE TABLE grabs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id             INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    download_client_id  INTEGER NOT NULL REFERENCES download_clients(id),
    title               TEXT NOT NULL,
    indexer             TEXT NOT NULL,
    guid                TEXT NOT NULL,
    size                INTEGER,
    download_url        TEXT NOT NULL,
    download_id         TEXT,
    status              TEXT NOT NULL DEFAULT 'sent',
    import_error        TEXT,
    grabbed_at          TEXT NOT NULL, media_type TEXT, content_path TEXT, import_retry_count INTEGER NOT NULL DEFAULT 0, import_failed_at TEXT,
    UNIQUE(user_id, guid, indexer)
);
CREATE TABLE history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id     INTEGER REFERENCES works(id) ON DELETE SET NULL,
    event_type  TEXT NOT NULL,
    data        TEXT NOT NULL DEFAULT '{}',
    date        TEXT NOT NULL
);
CREATE TABLE notifications (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    type        TEXT NOT NULL,
    ref_key     TEXT,
    message     TEXT NOT NULL,
    data        TEXT NOT NULL DEFAULT '{}',
    read        INTEGER NOT NULL DEFAULT 0,
    dismissed   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE TABLE indexers (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    name                      TEXT NOT NULL,
    url                       TEXT NOT NULL,
    api_path                  TEXT NOT NULL DEFAULT '/api',
    api_key                   TEXT,
    categories                TEXT NOT NULL DEFAULT '[7020,3030]',
    priority                  INTEGER NOT NULL DEFAULT 25,
    enable_automatic_search   INTEGER NOT NULL DEFAULT 1,
    enable_interactive_search INTEGER NOT NULL DEFAULT 1,
    supports_book_search      INTEGER NOT NULL DEFAULT 0,
    enabled                   INTEGER NOT NULL DEFAULT 1,
    added_at                  TEXT NOT NULL DEFAULT (datetime('now'))
, protocol TEXT NOT NULL DEFAULT 'torrent', enable_rss BOOLEAN NOT NULL DEFAULT 1);
CREATE TABLE author_bibliography (
    author_id INTEGER PRIMARY KEY REFERENCES authors(id) ON DELETE CASCADE,
    entries TEXT NOT NULL DEFAULT '[]',
    fetched_at TEXT NOT NULL DEFAULT (datetime('now'))
, raw_entries TEXT);
CREATE TABLE _livrarr_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
INSERT INTO _livrarr_meta VALUES('schema_version','34');
INSERT INTO _livrarr_meta VALUES('data_version','1');
INSERT INTO _livrarr_meta VALUES('identity_key_generation','1');
CREATE TABLE email_config (
    id              INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    enabled         BOOLEAN NOT NULL DEFAULT 0,
    smtp_host       TEXT NOT NULL DEFAULT 'smtp.gmail.com',
    smtp_port       INTEGER NOT NULL DEFAULT 587,
    encryption      TEXT NOT NULL DEFAULT 'starttls',
    username        TEXT,
    password        TEXT,
    from_address    TEXT,
    recipient_email TEXT,
    send_on_import  BOOLEAN NOT NULL DEFAULT 0
);
INSERT INTO email_config VALUES(1,0,'smtp.gmail.com',587,'starttls',NULL,NULL,NULL,NULL,0);
CREATE TABLE indexer_rss_state (
    indexer_id INTEGER PRIMARY KEY REFERENCES indexers(id) ON DELETE CASCADE,
    last_publish_date TEXT,
    last_guid TEXT
);
CREATE TABLE indexer_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    rss_sync_interval_minutes INTEGER NOT NULL DEFAULT 15,
    rss_match_threshold REAL NOT NULL DEFAULT 0.80
, rss_grab_failure_limit INTEGER NOT NULL DEFAULT 3);
INSERT INTO indexer_config VALUES(1,15,0.800000000000000044,3);
CREATE TABLE IF NOT EXISTS "playback_progress" (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_item_id INTEGER NOT NULL REFERENCES library_items(id) ON DELETE CASCADE,
    position TEXT NOT NULL,
    progress_pct REAL NOT NULL DEFAULT 0.0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')), finished_at TEXT,
    UNIQUE(user_id, library_item_id)
);
CREATE TABLE author_series_cache (
    author_id   INTEGER PRIMARY KEY REFERENCES authors(id) ON DELETE CASCADE,
    entries     TEXT NOT NULL DEFAULT '[]',
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now'))
, raw_entries TEXT);
CREATE TABLE IF NOT EXISTS "imports" (
    id                    TEXT    PRIMARY KEY,
    user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source                TEXT    NOT NULL DEFAULT 'readarr',
    status                TEXT    NOT NULL DEFAULT 'running'
                                  CHECK (status IN ('running', 'completed', 'failed', 'undone')),
    started_at            TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    completed_at          TEXT,
    authors_created       INTEGER NOT NULL DEFAULT 0,
    works_created         INTEGER NOT NULL DEFAULT 0,
    files_imported        INTEGER NOT NULL DEFAULT 0,
    files_skipped         INTEGER NOT NULL DEFAULT 0,
    source_url            TEXT,
    target_root_folder_id INTEGER REFERENCES root_folders(id)
);
CREATE TABLE IF NOT EXISTS "list_import_previews" (
    id             INTEGER PRIMARY KEY,
    preview_id     TEXT    NOT NULL,
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    row_index      INTEGER NOT NULL,
    title          TEXT    NOT NULL,
    author         TEXT    NOT NULL,
    isbn_13        TEXT,
    isbn_10        TEXT,
    year           INTEGER,
    source_status  TEXT,
    source_rating  REAL,
    preview_status TEXT    NOT NULL
                           CHECK (preview_status IN ('new', 'already_exists', 'parse_error')),
    source         TEXT    NOT NULL
                           CHECK (source IN ('goodreads', 'hardcover')),
    created_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
, goodreads_book_id TEXT);
CREATE TABLE IF NOT EXISTS "series" (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id           INTEGER NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    author_id         INTEGER NOT NULL REFERENCES authors(id)  ON DELETE CASCADE,
    name              TEXT    NOT NULL,
    gr_key            TEXT    NOT NULL,
    monitor_ebook     BOOLEAN NOT NULL DEFAULT FALSE,
    monitor_audiobook BOOLEAN NOT NULL DEFAULT FALSE,
    work_count        INTEGER NOT NULL DEFAULT 0,
    added_at          TEXT    NOT NULL DEFAULT (datetime('now')), monitor_language TEXT,
    UNIQUE(user_id, author_id, gr_key)
);
INSERT INTO series VALUES(1,1,1,'Earthsea Cycle','40909',0,0,1,'2026-06-01T12:00:00+00:00',NULL);
CREATE TABLE work_metadata_provenance (
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id   INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    field     TEXT NOT NULL,
    source    TEXT,
    set_at    TEXT NOT NULL,
    setter    TEXT NOT NULL,
    cleared   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (work_id, field)
);
INSERT INTO work_metadata_provenance VALUES(1,1,'title',NULL,'2026-06-01T12:00:00+00:00','user',0);
INSERT INTO work_metadata_provenance VALUES(1,1,'description','hardcover','2026-06-01T12:00:00+00:00','provider',0);
INSERT INTO work_metadata_provenance VALUES(1,2,'year','openlibrary','2026-06-01T12:00:00+00:00','provider',0);
CREATE TABLE provider_retry_state (
    user_id                INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id                INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    provider               TEXT NOT NULL,
    attempts               INTEGER NOT NULL DEFAULT 0,
    last_outcome           TEXT,
    last_attempt_at        TEXT,
    next_attempt_at        TEXT,
    normalized_payload_json TEXT,
    PRIMARY KEY (work_id, provider)
);
CREATE TABLE work_identity_anchors (
    work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    anchor_type   TEXT NOT NULL,
    anchor_value  TEXT NOT NULL,
    confidence    TEXT NOT NULL CHECK (confidence IN ('confirmed', 'pending', 'superseded')),
    setter        TEXT NOT NULL CHECK (setter IN ('user', 'auto_isbn', 'auto_search', 'import', 'redirect')),
    set_at        TEXT NOT NULL,
    superseded_by TEXT, user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (work_id, anchor_type, anchor_value)
);
INSERT INTO work_identity_anchors VALUES(1,'ol_work','/works/OL900001W','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(1,'hc_work','upgrade-fixture-900001','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(1,'gr_work','900001','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(1,'isbn_13','9780009000010','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(1,'asin','B0UPG00001','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(2,'ol_work','/works/OL900002W','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(2,'hc_work','upgrade-fixture-900002','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(2,'gr_work','900002','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(2,'isbn_13','9780009000027','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(2,'asin','B0UPG00002','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(3,'ol_work','/works/OL900003W','confirmed','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(3,'hc_work','upgrade-fixture-900003','confirmed','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(3,'gr_work','900003','confirmed','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(3,'isbn_13','9780009000034','confirmed','auto_isbn','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(3,'asin','B0UPG00003','confirmed','auto_isbn','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(4,'ol_work','/works/OL900004W','pending','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(4,'hc_work','upgrade-fixture-900004','pending','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(4,'gr_work','900004','pending','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(4,'isbn_13','9780009000041','pending','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(4,'asin','B0UPG00004','pending','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(5,'ol_work','','pending','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(5,'ol_work','/works/OL900055W','pending','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(5,'gr_work','900055','pending','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(7,'gr_work','900007','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(8,'gr_work','900008','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(9,'ol_work','/works/OL900009W','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(9,'hc_work','upgrade-fixture-900009','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(9,'gr_work','900009','confirmed','auto_search','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(9,'isbn_13','9780009000096','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(9,'asin','B0UPG00009','confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(13,'gr_work','900013','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(15,'ol_work',' /works/OL900015W','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(16,'ol_work',replace('	/works/OL900015W\n','\n',char(10)),'confirmed','import','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(18,'ol_work','/works/OL900018W','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(19,'ol_work','/works/OL900019W','confirmed','user','2026-06-01T12:00:00+00:00',NULL,1);
INSERT INTO work_identity_anchors VALUES(20,'ol_work','/works/OL900002W','confirmed','import','2026-06-01T12:00:00+00:00',NULL,2);
CREATE TABLE audiobook_chapters (
    id INTEGER PRIMARY KEY,
    library_item_id INTEGER NOT NULL REFERENCES library_items(id) ON DELETE CASCADE,
    chapter_index INTEGER NOT NULL,
    title TEXT NOT NULL,
    start_time_secs REAL NOT NULL,
    end_time_secs REAL NOT NULL,
    UNIQUE(library_item_id, chapter_index)
);
CREATE TABLE bookmarks (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    library_item_id INTEGER NOT NULL REFERENCES library_items(id) ON DELETE CASCADE,
    media_type TEXT NOT NULL,
    position TEXT NOT NULL,
    sort_key REAL NOT NULL,
    name TEXT NOT NULL,
    chapter_title TEXT,
    paired_bookmark_id INTEGER REFERENCES bookmarks(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE IF NOT EXISTS "work_identity_conflicts" (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    existing_work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    kind                  TEXT NOT NULL,
    incoming_payload_json TEXT NOT NULL,
    raised_at             TEXT NOT NULL,
    raised_by             TEXT NOT NULL,
    raised_source_path    TEXT,
    status                TEXT NOT NULL DEFAULT 'open',
    resolved_at           TEXT,
    resolution_action     TEXT,
    resolution_notes      TEXT
);
INSERT INTO work_identity_conflicts VALUES(1,1,18,'incoming_different_ol_key','{"ol_key":"/works/OL900098W","gr_key":null,"hc_key":null,"isbn_13":null,"asin":null,"title":"Neuromancer","author_name":"William Gibson","year":1984,"cover_url":null,"top_candidates":[]}','2026-06-01T12:00:00+00:00','refresh',NULL,'open',NULL,NULL,NULL);
INSERT INTO work_identity_conflicts VALUES(2,1,19,'incoming_different_ol_key','{"ol_key":"/works/OL900097W","gr_key":null,"hc_key":null,"isbn_13":null,"asin":null,"title":"Count Zero","author_name":"William Gibson","year":1984,"cover_url":null,"top_candidates":[]}','2026-06-01T12:00:00+00:00','refresh',NULL,'resolved','2026-06-02T08:00:00+00:00','keep_existing','kept my match');
INSERT INTO work_identity_conflicts VALUES(3,1,19,'incoming_different_ol_key','{"ol_key":"/works/OL900096W","gr_key":null,"hc_key":null,"isbn_13":null,"asin":null,"title":"Count Zero","author_name":"William Gibson","year":1984,"cover_url":null,"top_candidates":[]}','2026-06-01T12:00:00+00:00','refresh',NULL,'dismissed','2026-06-03T08:00:00+00:00',NULL,NULL);
CREATE TABLE provider_policy (
    language TEXT    NOT NULL,
    kind     TEXT    NOT NULL,
    provider TEXT    NOT NULL,
    rank     INTEGER NOT NULL,
    PRIMARY KEY (language, kind, provider)
);
INSERT INTO provider_policy VALUES('*','ebook','google_books',0);
INSERT INTO provider_policy VALUES('*','audiobook','audible',0);
INSERT INTO provider_policy VALUES('*','audiobook','audnexus',1);
CREATE TABLE kash_links (
    id INTEGER PRIMARY KEY,
    audio_item_id INTEGER NOT NULL UNIQUE REFERENCES library_items(id) ON DELETE CASCADE,
    ebook_item_id INTEGER NOT NULL UNIQUE REFERENCES library_items(id) ON DELETE CASCADE,
    container_duration_secs REAL NOT NULL,
    epub_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE cross_format_state (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kash_link_id INTEGER NOT NULL REFERENCES kash_links(id) ON DELETE CASCADE,
    furthest_ts REAL NOT NULL DEFAULT 0,
    ebook_declined_at_ts REAL,
    audio_declined_at_ts REAL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (user_id, kash_link_id)
);
CREATE TABLE provider_call_records (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider    TEXT    NOT NULL,
    operation   TEXT    NOT NULL,
    work_id     INTEGER,
    started_at  TEXT    NOT NULL,
    duration_ms INTEGER NOT NULL,
    outcome     TEXT    NOT NULL,
    detail      TEXT
);
CREATE TABLE work_field_dissents (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id          INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    provider         TEXT    NOT NULL,
    field            TEXT    NOT NULL,
    offered_value    TEXT    NOT NULL,
    winning_value    TEXT,
    reason           TEXT    NOT NULL,
    merge_generation INTEGER NOT NULL,
    recorded_at      TEXT    NOT NULL
);
CREATE TABLE series_roster (
    series_id   INTEGER PRIMARY KEY REFERENCES series(id) ON DELETE CASCADE,
    entries     TEXT NOT NULL DEFAULT '[]',
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE work_anchor_dead_ends (
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    anchor_type TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    PRIMARY KEY (work_id, anchor_type)
);
CREATE TABLE work_identity_review_candidates (
    work_id         INTEGER PRIMARY KEY REFERENCES works(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    candidates_json TEXT    NOT NULL,
    recorded_at     TEXT    NOT NULL
);
CREATE TABLE provider_response_cache (
    provider     TEXT NOT NULL,
    anchor_type  TEXT NOT NULL,
    anchor       TEXT NOT NULL,
    payload      TEXT NOT NULL,
    fetched_at   TEXT NOT NULL,
    PRIMARY KEY (provider, anchor_type, anchor)
);
DELETE FROM sqlite_sequence;
INSERT INTO sqlite_sequence VALUES('users',2);
INSERT INTO sqlite_sequence VALUES('naming_config',1);
INSERT INTO sqlite_sequence VALUES('media_management_config',1);
INSERT INTO sqlite_sequence VALUES('prowlarr_config',1);
INSERT INTO sqlite_sequence VALUES('metadata_config',1);
INSERT INTO sqlite_sequence VALUES('series',1);
INSERT INTO sqlite_sequence VALUES('work_identity_conflicts',3);
INSERT INTO sqlite_sequence VALUES('authors',20);
INSERT INTO sqlite_sequence VALUES('works',20);
CREATE UNIQUE INDEX idx_users_username_ci ON users(LOWER(username));
CREATE INDEX idx_authors_user_id ON authors(user_id);
CREATE INDEX idx_works_user_id ON works(user_id);
CREATE UNIQUE INDEX idx_root_folders_media_type ON root_folders(media_type);
CREATE INDEX idx_library_items_user_id ON library_items(user_id);
CREATE INDEX idx_grabs_user_id ON grabs(user_id);
CREATE INDEX idx_history_user_id ON history(user_id);
CREATE INDEX idx_history_date ON history(date);
CREATE INDEX idx_notifications_user_id ON notifications(user_id);
CREATE UNIQUE INDEX idx_notifications_dedup ON notifications(user_id, type, ref_key);
CREATE UNIQUE INDEX idx_download_clients_default_per_protocol
ON download_clients (client_type)
WHERE is_default_for_protocol = true;
CREATE INDEX idx_playback_progress_user ON playback_progress(user_id);
CREATE INDEX idx_works_series_id ON works(series_id);
CREATE INDEX idx_works_author_grkey ON works(author_id, gr_key);
CREATE INDEX idx_works_author_series_name ON works(author_id, series_name);
CREATE UNIQUE INDEX idx_imports_running ON imports(user_id) WHERE status = 'running';
CREATE INDEX idx_lip_preview_user ON list_import_previews(preview_id, user_id);
CREATE UNIQUE INDEX idx_lip_row ON list_import_previews(preview_id, user_id, row_index);
CREATE INDEX idx_series_user_author ON series(user_id, author_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);
CREATE UNIQUE INDEX uniq_primary_confirmed_anchor
    ON work_identity_anchors(work_id, anchor_type)
    WHERE confidence = 'confirmed';
CREATE INDEX idx_anchor_value
    ON work_identity_anchors(anchor_type, anchor_value);
CREATE UNIQUE INDEX uniq_user_confirmed_ol_anchor
    ON work_identity_anchors(user_id, anchor_type, anchor_value)
    WHERE confidence = 'confirmed';
CREATE INDEX idx_chapters_item ON audiobook_chapters(library_item_id);
CREATE INDEX idx_bookmarks_user_item ON bookmarks(user_id, library_item_id);
CREATE INDEX idx_bookmarks_paired ON bookmarks(paired_bookmark_id);
CREATE INDEX idx_identity_conflicts_user_status
    ON work_identity_conflicts(user_id, status);
CREATE INDEX idx_identity_conflicts_work
    ON work_identity_conflicts(existing_work_id);
CREATE INDEX idx_cross_format_state_link ON cross_format_state(kash_link_id);
CREATE INDEX idx_provider_call_records_started_at
    ON provider_call_records(started_at);
CREATE INDEX idx_provider_call_records_provider_started
    ON provider_call_records(provider, started_at);
CREATE INDEX idx_work_field_dissents_work
    ON work_field_dissents(user_id, work_id);
CREATE INDEX idx_works_convergence_due ON works(user_id, next_convergence_at);
CREATE INDEX idx_work_identity_review_candidates_user
    ON work_identity_review_candidates(user_id);
CREATE INDEX idx_provider_response_cache_fetched_at
    ON provider_response_cache(fetched_at);
CREATE UNIQUE INDEX idx_works_identity ON works(user_id, normalized_title, normalized_author);
COMMIT;
