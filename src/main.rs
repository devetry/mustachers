extern crate image;
extern crate imageproc;
extern crate rustface;

use std::cell::Cell;
use std::fs;
use std::io::Write;

use actix_multipart::{Field, Multipart, MultipartError};
use actix_web::{error, middleware, web, App, Error, HttpResponse, HttpServer};
use futures::future::{err, Either};
use futures::{Future, Stream};


use std::time::{Duration, Instant};

use image::{DynamicImage, GrayImage, Rgba, FilterType};
use image::imageops::overlay;
use imageproc::drawing::{draw_hollow_rect_mut};
use imageproc::rect::Rect;

use rustface::{Detector, FaceInfo, ImageData};

const OUTPUT_FILE: &str = "test.png";


pub struct AppState {
    pub counter: Cell<usize>,
}

pub fn save_file(field: Field) -> impl Future<Item = i64, Error = Error> {
    let file_path_string = "upload.png";
    let file = match fs::File::create(file_path_string) {
        Ok(file) => file,
        Err(e) => return Either::A(err(error::ErrorInternalServerError(e))),
    };
    Either::B(
        field
            .fold((file, 0i64), move |(mut file, mut acc), bytes| {
                // fs operations are blocking, we have to execute writes
                // on threadpool
                web::block(move || {
                    file.write_all(bytes.as_ref()).map_err(|e| {
                        println!("file.write_all failed: {:?}", e);
                        MultipartError::Payload(error::PayloadError::Io(e))
                    })?;
                    acc += bytes.len() as i64;
                    Ok((file, acc))
                })
                .map_err(|e: error::BlockingError<MultipartError>| {
                    match e {
                        error::BlockingError::Error(e) => e,
                        error::BlockingError::Canceled => MultipartError::Incomplete,
                    }
                })
            })
            .map(|(_, acc)| acc)
            .map_err(|e| {
                println!("save_file failed, {:?}", e);
                error::ErrorInternalServerError(e)
            }),
    )
}

fn get_millis(duration: Duration) -> u64 {
    duration.as_secs() * 1000u64 + u64::from(duration.subsec_nanos() / 1_000_000)
}

fn detect_faces(detector: &mut dyn Detector, gray: &GrayImage) -> Vec<FaceInfo> {
    let (width, height) = gray.dimensions();
    let mut image = ImageData::new(gray.as_ptr(), width, height);
    let now = Instant::now();
    let faces = detector.detect(&mut image);
    println!(
        "Found {} faces in {} ms",
        faces.len(),
        get_millis(now.elapsed())
    );
    faces
}


pub fn upload(
    multipart: Multipart,
    counter: web::Data<Cell<usize>>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    counter.set(counter.get() + 1);
    println!("{:?}", counter.get());



    multipart
        .map_err(error::ErrorInternalServerError)
        .map(|field| save_file(field).into_stream())
        .flatten()
        .collect()
        .map(|_sizes| {
            let mustache = match image::open("mustache_1.png") {
                Ok(image) => image,
                Err(message) => {
                    println!("Fialed to read image: {}", message);
                    std::process::exit(1)
                }
            };

            let image: DynamicImage = match image::open("upload.png") {
                Ok(image) => image,
                Err(message) => {
                    println!("Failed to read image: {}", message);
                    std::process::exit(1)
                }
            };

            let mut detector = match rustface::create_detector("model/seeta_fd_frontal_v1.0.bin") {
                Ok(detector) => detector,
                Err(error) => {
                    println!("Failed to create detector: {}", error.to_string());
                    std::process::exit(1)
                }
            };

            detector.set_min_face_size(20);
            detector.set_score_thresh(2.0);
            detector.set_pyramid_scale_factor(0.8);
            detector.set_slide_window_step(4, 4);


            let mut rgba = image.to_rgba();
            let faces = detect_faces(&mut *detector, &image.to_luma());

            for face in faces {
                let bbox = face.bbox();
                let rect = Rect::at(bbox.x(), bbox.y()).of_size(bbox.width(), bbox.height());
                draw_hollow_rect_mut(&mut rgba, rect, Rgba([255, 0, 0, 255]));
                let resized = mustache.resize(5 * bbox.width() / 10, 7 * bbox.height() / 10, FilterType::Nearest);
                let mustache_again = resized.to_rgba();
                // let resized: image::ImageBuffer<Rgb<u8>, Vec<u8>> =  ImageBuffer::from_pixel( 4 * bbox.width() / 10, bbox.height() / 10, Rgb([100,100,100]));
                overlay(&mut rgba, &mustache_again, bbox.x() as u32 + (5 * bbox.width() / 10) - mustache_again.width() / 2 as u32, bbox.y() as u32 + (7 * bbox.height() / 10));
            }

            match rgba.save(OUTPUT_FILE) {
                Ok(_) => println!("Saved result to {}", OUTPUT_FILE),
                Err(message) => println!("Failed to save result to a file. Reason: {}", message),
            }


            let transformed_image = fs::read(OUTPUT_FILE).expect("Unable to read image file");
            HttpResponse::Ok()
                .content_type("image/png")
                .body(transformed_image)
        })
        .map_err(|e| {
            println!("failed: {}", e);
            e
        })
}

fn index() -> HttpResponse {
    let html = r#"<html>
        <head><title>Upload a picture!</title></head>
        <body>
            <h3>Upload an image with people's faces in it!</h3>
            <form target="/" method="post" enctype="multipart/form-data">
                <input type="file" name="file"/>
                <input type="submit" value="Submit"></button>
            </form>
        </body>
    </html>"#;

    HttpResponse::Ok().body(html)
}

fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info");
    env_logger::init();

    HttpServer::new(|| {
        App::new()
            .data(Cell::new(0usize))
            .wrap(middleware::Logger::default())
            .service(
                web::resource("/")
                    .route(web::get().to(index))
                    .route(web::post().to_async(upload)),
            )
    })
    .bind("127.0.0.1:8080")?
    .run()
}