use std::{env, fs, process, thread};
use std::error::Error;
use async_recursion::async_recursion;
use hyper::header::HeaderValue;
use hyper::{Body, Client, Method, Request, Response};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;
use once_cell::sync::Lazy;

static mut CONFIG: Lazy<Config> = Lazy::new(|| {
    Config::read_config().unwrap_or_else(|e| {
        eprintln!("{}", e);
        process::exit(1)
    })
});

pub struct Config {
    pub block_size: u64,
    pub uri: String,
    pub path: String,
    pub file_name: String,
}

pub type Result<T, E = Box<dyn Error>> = core::result::Result<T, E>;

impl Config {
    pub fn read_config() -> Result<Self, &'static str> {
        let mut args = env::args();
        if args.len() < 5 {
            return Err("参数小于 4 位");
        }
        args.next();
        let block_size = match args.next() {
            Some(arg) => arg.parse::<u64>().unwrap(),
            None => return Err("参数 block_size 不存在")
        };
        let uri = match args.next() {
            Some(arg) => arg,
            None => return Err("参数 uri 不存在")
        };
        let path = match args.next() {
            Some(arg) => arg,
            None => return Err("参数 path 不存在")
        };
        let file_name = match args.next() {
            Some(arg) => arg,
            None => return Err("参数 file_name 不存在")
        };
        Ok(Self { block_size, uri, path, file_name })
    }
    pub fn set_uri(&mut self, url: &String) {
        self.uri = url.to_string();
    }
}

/// 根据数据开始、结束索引分段下载文件。
/// 文件名称后缀为 index。如：test.zip1。
async fn download_block(index: u64, start: u64, end: u64) -> Result<()> {
    // let client = Client::new();
    // let request = Request::builder()
    //     .method(Method::GET)
    //     .header("range", format!("bytes={}-{}", start, end))
    //     .uri(&CONFIG.uri)
    //     .body(Body::empty())?;
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);
    unsafe {
        let request = Request::builder()
            .method(Method::GET)
            .header("range", format!("bytes={}-{}", start, end))
            .uri(&CONFIG.uri)
            .body(Body::empty())?;

        let response = client.request(request).await?;
        write_file(response, &(CONFIG.path.clone() + &CONFIG.file_name + &index.to_string())).await?;
    }

    Ok(())
}

async fn download(client: Client<HttpsConnector<HttpConnector>, Body>) -> Result<()> {
    unsafe {
        let response = client.get(CONFIG.uri.parse()?).await?;
        write_file(response, &(CONFIG.path.clone() + &CONFIG.file_name)).await?;
    }
    Ok(())
}

/// 写入文件
async fn write_file(response: Response<Body>, file_name: &str) -> Result<()> {
    let body = hyper::body::to_bytes(response).await?;
    fs::write(file_name, body.iter())?;
    println!("write {}", file_name);
    Ok(())
}

/// 合并文件
fn merge(size: u64) -> Result<()> {
    unsafe {
        let dirs = fs::read_dir(&CONFIG.path)?;
        let file_name = CONFIG.path.clone() + &CONFIG.file_name;
        let mut result = vec![];
        for dir in dirs {
            let path = dir?.path();
            let path_str = path.to_str().unwrap();
            if path_str.starts_with(&file_name) {
                result.extend_from_slice(&fs::read(&path)?);
                // 删除临时文件
                fs::remove_file(&path)?;
                println!("remove {}", path_str)
            }
        }
        if size as usize == result.len() {
            println!("合并文件完成");
            fs::write(CONFIG.path.clone() + &CONFIG.file_name, result)?
        }
    }
    Ok(())
}

#[async_recursion]
pub async fn run() -> Result<()> {
    unsafe {
        // let client = Client::new();
        let https = HttpsConnector::new();
        let client = Client::builder().build::<_, hyper::Body>(https);
        let request = Request::builder()
            .method(Method::HEAD)
            .uri(&CONFIG.uri)
            .body(Body::empty())?;
        let response = client.request(request).await?;
        let headers = response.headers();
        let accept_ranges = headers.get("accept-ranges");
        // println!("{:#?}", accept_ranges.unwrap());
        let content_length = headers.get("content-length");
        println!("{:#?}", content_length.unwrap());
        let redirected_url = headers.get("Location");
        // let v = &HeaderValue::from_str("bytes").unwrap();
        if let Some(url) = redirected_url {
            println!("{:?}", url);
            CONFIG.set_uri(&url.to_str().unwrap().to_string());
            // accept_ranges = Some(v);
            run().await?;
            std::process::exit(0)
        }
        let accept_ranges_flag = match accept_ranges {
            None => false,
            Some(v) => v.to_str()?.eq("bytes")
        };
        if accept_ranges_flag & &content_length.clone().is_some() {
            println!("支持并发下载");
            let size = content_length.unwrap().to_str()?.parse::<u64>()?;
            if size == 0 {
                println!("数据为空");
                return Ok(());
            }
            let t_size = size / CONFIG.block_size;
            if t_size <= 1 {
                println!("数据分片 <= 1，单线程下载");
                download(client).await?;
                return Ok(());
            }
            let first_attach = size % CONFIG.block_size;
            println!("数据块长度 {}", size);
            println!("启用 {} 个线程下载", t_size);
            let mut ts = vec![
                thread::spawn(move || {
                    println!("线程 1 启动");
                    download_block(0, 0, CONFIG.block_size - 1 + first_attach)
                })
            ];
            for i in 1..t_size {
                let t = thread::spawn(move || {
                    let start = i * CONFIG.block_size + first_attach;
                    println!("线程 {} 启动", i + 1);
                    download_block(i, start, start + CONFIG.block_size - 1)
                });
                ts.push(t)
            }
            // 等待所有线程结束
            for t in ts {
                t.join().unwrap().await?;
            }
            println!("下载完成，开始合并文件");
            merge(size)?;
        } else {
            println!("不支持并发下载");
            download(client).await?;
        }
    }
    Ok(())
}
