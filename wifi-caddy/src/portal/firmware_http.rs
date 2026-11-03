//! Firmware upload endpoint: the page and the upload loop.
//!
//! The endpoint accepts the raw image as the request body, so the device never
//! parses a multipart envelope: the page posts the chosen file with
//! `XMLHttpRequest`, and `curl --data-binary @image.bin http://<board>/firmware`
//! works as well.
//!
//! The upload holds the config storage lock for its duration. That is the only
//! lock involved, and it is what keeps a configuration write from interleaving
//! with the flash operations of an update.

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::mutex::Mutex;
use edge_http::io::Error;
use edge_http::io::server::Connection;
use embedded_io_async::{Read, Write};

use super::firmware::{FirmwareError, FirmwareSink};
use super::responses::send_text;

/// Largest chunk read from the connection.
///
/// The reading side's buffer is what limits a single read, so a larger chunk only
/// costs stack without shortening the upload.
const RX_CHUNK: usize = 1024;

/// Page served by `GET /firmware`.
///
/// Choosing a file starts the upload, so there is no second button to press: a
/// captive-portal window, which some browsers open when joining the device's
/// network, blocks file dialogs, and the hint at the bottom names the way past it.
const PAGE: &str = r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Firmware update</title><style>
body{font-family:system-ui,sans-serif;margin:0 auto;max-width:34rem;padding:1.5rem;color:#111}
h1{font-size:1.3rem}p{margin:.8rem 0}progress{width:100%;height:1.4rem}
#msg{white-space:pre-line}#msg.bad{color:#b00}
.hint{font-size:.85rem;color:#555}code{font-size:.85rem}
</style></head><body>
<h1>Firmware update</h1>
<p>Choose the binary image built for this board. The device verifies it and
reboots into it, so keep this page open until it says the update was accepted.</p>
<p><input type="file" id="file" accept=".bin,application/octet-stream"></p>
<progress id="bar" value="0" max="100" hidden></progress>
<p id="msg">No image chosen yet.</p>
<p><a href="/">Back to configuration</a></p>
<p class="hint">If this window will not open a file dialog - a captive-portal
window usually does not - post the image directly:<br>
<code>curl --data-binary @image.bin http://&lt;portal address&gt;/firmware</code></p>
<script>
var file=document.getElementById('file'),bar=document.getElementById('bar');
var msg=document.getElementById('msg');
function say(text,bad){msg.textContent=text;msg.className=bad?'bad':'';}
file.addEventListener('change',function(){
  if(!file.files.length){say('No image chosen yet.');return;}
  var image=file.files[0],req=new XMLHttpRequest();
  bar.hidden=false;bar.value=0;
  say('Uploading '+image.name+' ('+image.size+' bytes) - keep this page open.');
  req.upload.onprogress=function(e){if(e.lengthComputable){bar.value=Math.round(e.loaded*100/e.total);}};
  req.onload=function(){
    if(req.status===200){say('Update accepted. The device is rebooting into the new firmware.');}
    else{say('Failed ('+req.status+'): '+req.responseText,true);}
  };
  req.onerror=function(){say('Upload failed. Check that the device is still reachable.',true);};
  req.open('POST','/firmware');req.send(image);
});
</script></body></html>"#;

/// Serves the upload page.
pub async fn serve_page<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
) -> Result<(), Error<T::Error>>
where
    T: Read + Write,
{
    conn.initiate_response(
        200,
        None,
        &[
            ("Content-Type", "text/html; charset=utf-8"),
            ("Connection", "close"),
        ],
    )
    .await?;
    conn.write_all(PAGE.as_bytes()).await?;
    conn.complete().await
}

/// Streams the request body into the sink.
///
/// The storage lock is held for the whole upload rather than per chunk, so no
/// configuration write can interleave with the flash operations of an update. A
/// client that stops sending releases it: every connection carries the portal's
/// IO timeout, and every error path aborts the partially written image.
pub async fn run_upload<R, S, F, T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
    io: &'static Mutex<R, S>,
    sink: &F,
) -> Result<(), Error<T::Error>>
where
    R: RawMutex,
    S: 'static,
    F: FirmwareSink<S>,
    T: Read + Write,
{
    // Copy the head into plain values: `headers` borrows the connection, and the
    // body read below needs it mutably.
    let (len, multipart) = {
        let headers = conn.headers()?;
        let len = headers
            .headers
            .content_len()
            .map(|len| len.min(u32::MAX as u64) as u32);
        let multipart = headers
            .headers
            .content_type()
            .is_some_and(|content_type| content_type.starts_with("multipart/"));
        (len, multipart)
    };

    if multipart {
        return send_text(
            conn,
            415,
            "Send the image as the raw request body, not as a form upload",
        )
        .await;
    }

    let mut io = io.lock().await;

    if let Err(error) = sink.begin(&mut *io, len).await {
        let status = match error {
            FirmwareError::Disabled => 404,
            FirmwareError::Rejected => 400,
            FirmwareError::Backend => 500,
        };
        drop(io);
        return send_text(conn, status, "Firmware update unavailable").await;
    }

    let mut rx = [0u8; RX_CHUNK];
    loop {
        let read = match conn.read(&mut rx).await {
            Ok(read) => read,
            Err(error) => {
                // The client gave up or the IO timeout fired: drop the partial
                // image so the next upload starts from an erased slot.
                sink.abort(&mut *io).await;
                return Err(error);
            }
        };

        if read == 0 {
            break;
        }

        if sink.write(&mut *io, &rx[..read]).await.is_err() {
            error!("http: firmware upload failed");
            sink.abort(&mut *io).await;
            drop(io);
            return send_text(conn, 500, "Firmware upload failed").await;
        }
    }

    match sink.finish(&mut *io).await {
        Ok(()) => {
            drop(io);
            send_text(conn, 200, "Update accepted, the device is rebooting").await
        }
        Err(_) => {
            error!("http: firmware verification failed");
            sink.abort(&mut *io).await;
            drop(io);
            send_text(conn, 500, "Firmware verification failed").await
        }
    }
}

