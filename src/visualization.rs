// src/visualization.rs
use petgraph::dot::{Config, Dot};
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use url::Url;

use crate::error::{CrawlerError, Result};

#[derive(Debug)]
pub struct GraphVisualizer {
    graph: DiGraph<String, ()>,
    node_map: HashMap<String, NodeIndex>,
}

impl GraphVisualizer {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_map: HashMap::new(),
        }
    }

    pub fn build_from_crawler_graph(&mut self, crawler_graph: &HashMap<String, Vec<String>>) {
        // Clear existing data
        self.graph = DiGraph::new();
        self.node_map.clear();

        // First pass: add all nodes
        for url in crawler_graph.keys() {
            self.get_or_create_node(url);
        }

        // Second pass: add all edges
        for (source, targets) in crawler_graph {
            let source_idx = self.node_map[source];
            for target in targets {
                if let Some(&target_idx) = self.node_map.get(target) {
                    self.graph.add_edge(source_idx, target_idx, ());
                }
            }
        }
    }

    fn get_or_create_node(&mut self, url: &str) -> NodeIndex {
        if let Some(&idx) = self.node_map.get(url) {
            return idx;
        }

        // Create a display name for the node (domain + path)
        let display_name = if let Ok(parsed) = Url::parse(url) {
            let domain = parsed.host_str().unwrap_or("unknown");
            let path = parsed.path();
            format!("{}{}", domain, path)
        } else {
            url.to_string()
        };

        let idx = self.graph.add_node(display_name);
        self.node_map.insert(url.to_string(), idx);
        idx
    }

    pub fn export_dot(&self, path: &str) -> Result<()> {
        let dot = format!(
            "{:?}",
            Dot::with_config(&self.graph, &[Config::EdgeNoLabel])
        );

        let mut file = File::create(path).map_err(|e| {
            CrawlerError::VisualizationError(format!("Failed to create DOT file: {}", e))
        })?;

        file.write_all(dot.as_bytes()).map_err(|e| {
            CrawlerError::VisualizationError(format!("Failed to write DOT file: {}", e))
        })?;

        Ok(())
    }

    pub fn export_html(&self, path: &str) -> Result<()> {
        // Convert the graph to a format suitable for D3.js visualization
        let mut nodes = Vec::new();
        let mut links = Vec::new();

        // Create nodes
        for (url, &idx) in &self.node_map {
            let node_data = self.graph[idx].clone();
            nodes.push(format!(
                "{{\"id\": {}, \"url\": \"{}\", \"name\": \"{}\"}}",
                idx.index(),
                url,
                node_data
            ));
        }

        // Create links
        for edge in self.graph.edge_indices() {
            let (source, target) = self.graph.edge_endpoints(edge).unwrap();
            links.push(format!(
                "{{\"source\": {}, \"target\": {}}}",
                source.index(),
                target.index()
            ));
        }

        // Create the HTML template with embedded D3.js visualization
        // Fix: Use raw string literals to avoid prefix issues
        let html = format!(
            r###"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Web Crawler Visualization</title>
    <script src="https://d3js.org/d3.v5.min.js"></script>
    <style>
        body {{ margin: 0; font-family: Arial, sans-serif; }}
        .links line {{
            stroke: #999;
            stroke-opacity: 0.6;
        }}
        .nodes circle {{
            stroke: #fff;
            stroke-width: 1.5px;
        }}
        .tooltip {{
            position: absolute;
            background: #f9f9f9;
            border: 1px solid #ccc;
            border-radius: 5px;
            padding: 10px;
            pointer-events: none;
            opacity: 0;
            transition: opacity 0.3s;
        }}
        .controls {{
            position: absolute;
            top: 10px;
            left: 10px;
            background: rgba(255, 255, 255, 0.8);
            padding: 10px;
            border-radius: 5px;
            border: 1px solid #ccc;
        }}
    </style>
</head>
<body>
    <div id="graph"></div>
    <div class="tooltip"></div>
    <div class="controls">
        <h3>Web Crawler Graph</h3>
        <div>
            <button id="zoom-in">+</button>
            <button id="zoom-out">-</button>
            <button id="reset">Reset</button>
        </div>
        <div>
            <p><span id="node-count">0</span> pages, <span id="link-count">0</span> links</p>
        </div>
    </div>
    
    <script>
        const width = window.innerWidth;
        const height = window.innerHeight;
        
        const nodes = [{nodes}];
        const links = [{links}];
        
        document.getElementById('node-count').textContent = nodes.length;
        document.getElementById('link-count').textContent = links.length;
        
        const svg = d3.select("#graph")
            .append("svg")
            .attr("width", width)
            .attr("height", height);
            
        const g = svg.append("g");
        
        // Add zoom behavior
        const zoom = d3.zoom()
            .scaleExtent([0.1, 10])
            .on("zoom", () => {{
                g.attr("transform", d3.event.transform);
            }});
            
        svg.call(zoom);
        
        // Zoom controls
        d3.select("#zoom-in").on("click", () => {{
            svg.transition().call(zoom.scaleBy, 1.3);
        }});
        
        d3.select("#zoom-out").on("click", () => {{
            svg.transition().call(zoom.scaleBy, 0.7);
        }});
        
        d3.select("#reset").on("click", () => {{
            svg.transition().call(zoom.transform, d3.zoomIdentity);
        }});
            
        const simulation = d3.forceSimulation(nodes)
            .force("link", d3.forceLink(links).id(d => d.id).distance(100))
            .force("charge", d3.forceManyBody().strength(-300))
            .force("center", d3.forceCenter(width / 2, height / 2))
            .force("collide", d3.forceCollide().radius(30));
            
        const link = g.append("g")
            .attr("class", "links")
            .selectAll("line")
            .data(links)
            .enter().append("line")
            .attr("stroke-width", 1);
            
        const node = g.append("g")
            .attr("class", "nodes")
            .selectAll("circle")
            .data(nodes)
            .enter().append("circle")
            .attr("r", 5)
            .attr("fill", (d, i) => d3.schemeCategory10[i % 10])
            .call(d3.drag()
                .on("start", dragstarted)
                .on("drag", dragged)
                .on("end", dragended));
                
        const tooltip = d3.select(".tooltip");
        
        node.on("mouseover", function(d) {{
            tooltip.transition()
                .duration(200)
                .style("opacity", .9);
            tooltip.html(`<strong>${{d.name}}</strong><br>${{d.url}}`)
                .style("left", (d3.event.pageX + 10) + "px")
                .style("top", (d3.event.pageY - 28) + "px");
        }})
        .on("mouseout", function() {{
            tooltip.transition()
                .duration(500)
                .style("opacity", 0);
        }});
                
        simulation.on("tick", () => {{
            link
                .attr("x1", d => d.source.x)
                .attr("y1", d => d.source.y)
                .attr("x2", d => d.target.x)
                .attr("y2", d => d.target.y);
                
            node
                .attr("cx", d => d.x)
                .attr("cy", d => d.y);
        }});
        
        function dragstarted(d) {{
            if (!d3.event.active) simulation.alphaTarget(0.3).restart();
            d.fx = d.x;
            d.fy = d.y;
        }}
        
        function dragged(d) {{
            d.fx = d3.event.x;
            d.fy = d3.event.y;
        }}
        
        function dragended(d) {{
            if (!d3.event.active) simulation.alphaTarget(0);
            d.fx = null;
            d.fy = null;
        }}
    </script>
</body>
</html>"###,
            nodes = nodes.join(","),
            links = links.join(",")
        );

        let mut file = File::create(path).map_err(|e| {
            CrawlerError::VisualizationError(format!("Failed to create HTML file: {}", e))
        })?;

        file.write_all(html.as_bytes()).map_err(|e| {
            CrawlerError::VisualizationError(format!("Failed to write HTML file: {}", e))
        })?;

        Ok(())
    }
}

