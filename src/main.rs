use std::fs;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use serveur::client::{Client, VariationRequete};
use serveur::nom_fichier_date;
use serveur::ogn::thread_ogn;
use serveur::planche::{MettreAJour, MiseAJour, Planche};
use serveur::vol::Vol;

use chrono::{Datelike, NaiveDate, Utc};

use serveur::creer_chemin_jour;
use simple_http_parser::request;

fn main() {
    let date_aujourdhui = NaiveDate::from_ymd_opt(2023, 04, 25).unwrap();
    let requetes_en_cours: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
    let ecouteur = TcpListener::bind("127.0.0.1:7878").unwrap();

    let planche_arc: Arc<Mutex<Planche>> = Arc::new(Mutex::new(Planche::new()));
    let mut planche_lock = planche_arc.lock().unwrap();
    *planche_lock = Planche::planche_du(date_aujourdhui);
    drop(planche_lock);

    let planche_thread = planche_arc.clone();

    // creation du dossier de travail si besoin
    let mut chemins = fs::read_dir("./").unwrap();
    if !(chemins.any(|chemin| {
        chemin.unwrap().path().to_str().unwrap().to_string() == "./dossier_de_travail"
    })) {
        fs::create_dir(format!("./dossier_de_travail")).unwrap();
    }

    //on spawn le thread qui va s'occuper de ogn
    thread::spawn(move || {
        thread_ogn(planche_thread);
    });

    for flux in ecouteur.incoming() {
        let flux = flux.unwrap();
        let requetes_en_cours = requetes_en_cours.clone();

        let planche_arc = planche_arc.clone();

        thread::spawn(move || {
            gestion_connexion(flux, requetes_en_cours, planche_arc);
        });
    }
}

fn gestion_connexion(
    mut flux: TcpStream,
    requetes_en_cours: Arc<Mutex<Vec<Client>>>,
    planche: Arc<Mutex<Planche>>,
) {
    let adresse = format!("{}", (flux.peer_addr().unwrap()));

    requetes_en_cours.clone().incrementer(adresse.clone());

    let mut tampon = [0; 16384];
    flux.read(&mut tampon).unwrap();

    let requete_brute = String::from_utf8_lossy(&tampon).to_owned();
    let requete_parse = request::Request::from(&requete_brute)
        .expect("La requête n'a pas pu être parsé correctement.");
    let chemin = requete_parse.path;
    let corps_json = requete_parse.body.clone();
    let nom_fichier = match chemin.as_str() {
        "/" => "./planche/example.html",
        string => string,
    };

    let mut ligne_statut = "HTTP/1.1 200 OK";
    let mut headers = String::new();

    let contenu: String = match requete_parse.method {
        request::HTTPMethod::GET => {
            if &nom_fichier[1..5] != "vols" {
                if nom_fichier[nom_fichier.len() - 5..nom_fichier.len()].to_string()
                    == ".json".to_string()
                {
                    headers.push_str(
                        "Content-Type: application/json\
                        \nAccess-Control-Allow-Origin: *",
                    );
                }
                fs::read_to_string(format!("./parametres{}", nom_fichier)).unwrap_or_else(|_| {
                    ligne_statut = "HTTP/1.1 404 NOT FOUND";
                    fs::read_to_string("./parametres/planche/404.html").unwrap_or_else(|err| {
                        eprintln!("pas de 404.html !! : {}", err);
                        "".to_string()
                    })
                })
            } else if &(nom_fichier[0..5]) == "/vols" {
                headers.push_str(
                    "Content-Type: application/json\
                    \nAccess-Control-Allow-Headers: origin, content-type\
                    \nAccess-Control-Allow-Origin: *",
                );
                let date_aujourdhui = NaiveDate::from_ymd_opt(2023, 04, 25).unwrap();
                let date_str = &nom_fichier[5..16];
                let date = NaiveDate::parse_from_str(date_str, "/%Y/%m/%d").unwrap();

                if date != date_aujourdhui {
                    Planche::planche_du(date).vers_json()
                } else {
                    //on recupere la liste de planche
                    let planche_lock = planche.lock().unwrap();
                    let clone_planche = (*planche_lock).clone();
                    drop(planche_lock);
                    clone_planche.vers_json()
                }
            } else {
                "".to_string()
            }
        }

        request::HTTPMethod::POST => {
            if nom_fichier == "/mise_a_jour" {
                // les trois champs d'une telle requete sont séparés par des virgules tels que: "4,decollage,12:24,"
                let mut mise_a_jour = MiseAJour::new();
                let mut corps_json_nettoye = String::new(); //necessite de creer une string qui va contenir
                                                            //seulement les caracteres valies puisque le parser retourne des UTF0000 qui sont invalides pour le parser json
                for char in corps_json.chars() {
                    if char as u32 != 0 {
                        corps_json_nettoye.push_str(char.to_string().as_str());
                    }
                }

                mise_a_jour
                    .parse(json::parse(&corps_json_nettoye).unwrap())
                    .unwrap();
                let date_aujourdhui = NaiveDate::from_ymd_opt(2023, 04, 25).unwrap();

                if mise_a_jour.date != date_aujourdhui {
                    let mut planche_voulue = Planche::planche_du(mise_a_jour.date);
                    planche_voulue.mettre_a_jour(mise_a_jour);
                    planche_voulue.enregistrer();
                } else {
                    let mut planche_lock = planche.lock().unwrap();
                    (*planche_lock).mettre_a_jour(mise_a_jour);
                    (*planche_lock).enregistrer();
                    drop(planche_lock);
                }

                ligne_statut = "HTTP/1.1 201 Created";

                headers.push_str(
                    "Content-Type: application/json\
                    \nAccess-Control-Allow-Origin: *",
                );

                String::from("")
            } else {
                String::from("")
            }
        }

        request::HTTPMethod::OPTIONS => {
            if nom_fichier == "/mise_a_jour" {
                // les trois champs d'une telle requete sont séparés par des virgules tels que: "4,decollage,12:24,"
                ligne_statut = "HTTP/1.1 204 No Content";

                headers.push_str(
                    "Connection: keep-alive\
                    \nAccess-Control-Allow-Origin: *\
                    \nAccess-Control-Max-Age: 86400\
                    \nAccess-Control-Allow-Methods: POST, OPTIONS\
                    \nAccess-Control-Allow-headers: origin,  content-type",
                );

                String::from("")
            } else {
                String::from("")
            }
        }
        _ => String::from(""),
    };

    let reponse = format!(
        "{}\r\nContent-Length: {}\n\
        {}\r\n\r\n{}",
        ligne_statut,
        contenu.len(),
        headers,
        contenu
    );

    flux.write(reponse.as_bytes()).unwrap();
    flux.flush().unwrap();

    requetes_en_cours.clone().decrementer(adresse);
}

fn vols_enregistres_chemin(chemin_jour: String) -> Vec<Vol> {
    let fichiers_vols = fs::read_dir(format!("./dossier_de_travail/{}", chemin_jour)).unwrap();
    let mut vols: Vec<Vol> = Vec::new();

    for vol in fichiers_vols {
        let nom_fichier = vol.unwrap().path().to_str().unwrap().to_string();
        let fichier_vol_str = fs::read_to_string(format!("{}", nom_fichier)).unwrap();
        let vol_json_parse = Vol::depuis_json(json::parse(fichier_vol_str.as_str()).unwrap());
        vols.push(vol_json_parse);
    }
    vols
}

mod tests;
