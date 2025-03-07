// src/visualization.rs
use petgraph::dot::{Config, Dot};
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};
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

    // New method with performance optimizations
    pub fn export_html_optimized(
        &self,
        path: &str,
        max_nodes: usize,
        max_links_per_node: usize,
    ) -> Result<()> {
        // For very large graphs, we need to limit what we display
        let total_nodes = self.node_map.len();
        let node_limit = max_nodes.min(total_nodes);

        // Calculate importance of nodes (by number of connections)
        let mut node_importance: Vec<_> = self
            .node_map
            .iter()
            .map(|(url, &idx)| {
                let in_degree = self
                    .graph
                    .neighbors_directed(idx, petgraph::Direction::Incoming)
                    .count();
                let out_degree = self
                    .graph
                    .neighbors_directed(idx, petgraph::Direction::Outgoing)
                    .count();
                (url, idx, in_degree + out_degree)
            })
            .collect();

        // Sort by importance (highest connection count first)
        node_importance.sort_by(|(_, _, count1), (_, _, count2)| count2.cmp(count1));

        // Take only the most important nodes
        let selected_nodes: Vec<_> = node_importance.into_iter().take(node_limit).collect();

        // Create a set of selected node indices for quick lookup
        let selected_indices: HashSet<_> = selected_nodes.iter().map(|(_, idx, _)| *idx).collect();

        // Create nodes array for visualization
        let mut nodes = Vec::new();

        for (url, idx, _) in &selected_nodes {
            let node_data = self.graph[*idx].clone();

            // Extract domain for coloring
            let domain = if let Ok(parsed) = Url::parse(url) {
                parsed.host_str().unwrap_or("unknown").to_string()
            } else {
                "unknown".to_string()
            };

            nodes.push(format!(
                r#"{{"id": {}, "url": "{}", "name": "{}", "domain": "{}"}}"#,
                idx.index(),
                url,
                node_data,
                domain
            ));
        }

        // Create links (only between selected nodes and limited per node)
        let mut links = Vec::new();
        let mut links_per_node: HashMap<NodeIndex, usize> = HashMap::new();

        for &(_, source_idx, _) in &selected_nodes {
            let mut link_count = 0;

            for target_idx in self
                .graph
                .neighbors_directed(source_idx, petgraph::Direction::Outgoing)
            {
                if selected_indices.contains(&target_idx) && link_count < max_links_per_node {
                    links.push(format!(
                        r#"{{"source": {}, "target": {}}}"#,
                        source_idx.index(),
                        target_idx.index()
                    ));

                    *links_per_node.entry(source_idx).or_insert(0) += 1;
                    link_count += 1;
                }
            }
        }

        // Create the HTML template with optimized D3.js visualization
        let html = format!(
            r###"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Web Crawler Visualization (Optimized)</title>
    <script src="https://d3js.org/d3.v5.min.js"></script>
    <script src="https://d3js.org/d3-scale-chromatic.v1.min.js"></script>
    <style>
        body {{ 
            margin: 0; 
            font-family: Arial, sans-serif;
            overflow: hidden;
        }}
        #graph-container {{
            position: relative;
            width: 100vw;
            height: 100vh;
        }}
        canvas {{
            position: absolute;
            top: 0;
            left: 0;
        }}
        .controls {{
            position: absolute;
            top: 10px;
            left: 10px;
            background: rgba(255, 255, 255, 0.8);
            padding: 10px;
            border-radius: 5px;
            border: 1px solid #ccc;
            z-index: 10;
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
            z-index: 20;
        }}
        .domain-filters {{
            margin-top: 10px;
            max-height: 200px;
            overflow-y: auto;
        }}
        .filter-item {{
            margin: 5px 0;
        }}
        button {{
            margin: 0 5px;
            padding: 5px 10px;
            cursor: pointer;
        }}
    </style>
</head>
<body>
    <div id="graph-container"></div>
    <div class="tooltip"></div>
    <div class="controls">
        <h3>Web Crawler Graph</h3>
        <div>
            <button id="zoom-in">+</button>
            <button id="zoom-out">-</button>
            <button id="reset">Reset</button>
        </div>
        <div>
            <p>Showing top <strong>{node_limit}</strong> pages out of <strong>{total_nodes}</strong> total</p>
            <p><strong id="visible-nodes">{node_limit}</strong> nodes, <strong id="visible-links">{total_links}</strong> links visible</p>
        </div>
        <div>
            <label for="domain-select">Filter by Domain:</label>
            <select id="domain-select">
                <option value="all">All Domains</option>
            </select>
        </div>
        <div>
            <label for="render-quality">Performance Mode:</label>
            <select id="render-quality">
                <option value="high">High Quality</option>
                <option value="medium" selected>Balanced</option>
                <option value="low">Performance</option>
            </select>
        </div>
        <div class="domain-filters" id="domain-checkboxes"></div>
    </div>
    
    <script>
        // Performance optimization trick: use canvas instead of SVG for large graphs
        const width = window.innerWidth;
        const height = window.innerHeight;
        
        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        document.getElementById('graph-container').appendChild(canvas);
        
        const context = canvas.getContext('2d');
        
        // Parse nodes and links
        const rawNodes = [{nodes}];
        const rawLinks = [{links}];
        
        // Set up force simulation
        const simulation = d3.forceSimulation()
            .force("link", d3.forceLink().id(d => d.id))
            .force("charge", d3.forceManyBody().strength(-30))
            .force("center", d3.forceCenter(width / 2, height / 2))
            .force("collision", d3.forceCollide().radius(5))
            .alphaTarget(0);
            
        // Create domain color scale
        const allDomains = [...new Set(rawNodes.map(d => d.domain))];
        const color = d3.scaleOrdinal(d3.schemeCategory10).domain(allDomains);
        
        // Set up zoom behavior
        let transform = {{k: 1, x: 0, y: 0}};
        
        function zoomed() {{
            transform = d3.event.transform;
            render();
        }}
        
        const zoom = d3.zoom()
            .scaleExtent([0.1, 8])
            .on("zoom", zoomed);
            
        d3.select(canvas).call(zoom);
        
        // Set up node drag behavior
        let draggedNode = null;
        
        function dragStart(d) {{
            if (!d3.event.active) simulation.alphaTarget(0.3).restart();
            d.fx = d.x;
            d.fy = d.y;
            draggedNode = d;
        }}
        
        function dragged() {{
            if (draggedNode) {{
                const mouseX = (d3.event.x - transform.x) / transform.k;
                const mouseY = (d3.event.y - transform.y) / transform.k;
                draggedNode.fx = mouseX;
                draggedNode.fy = mouseY;
                simulation.restart();
            }}
        }}
        
        function dragEnd() {{
            if (!d3.event.active) simulation.alphaTarget(0);
            if (draggedNode) {{
                draggedNode.fx = null;
                draggedNode.fy = null;
                draggedNode = null;
            }}
        }}
        
        d3.select(canvas)
            .on('mousedown', () => {{
                const mouseX = (d3.event.offsetX - transform.x) / transform.k;
                const mouseY = (d3.event.offsetY - transform.y) / transform.k;
                
                // Find node under cursor
                const node = simulation.nodes().find(n => {{
                    const dx = n.x - mouseX;
                    const dy = n.y - mouseY;
                    return Math.sqrt(dx * dx + dy * dy) < 10;
                }});
                
                if (node) {{
                    dragStart(node);
                }}
            }})
            .on('mousemove', dragged)
            .on('mouseup', dragEnd);
        
        // Tooltip functionality
        const tooltip = d3.select(".tooltip");
        
        d3.select(canvas).on('mousemove', () => {{
            const mouseX = (d3.event.offsetX - transform.x) / transform.k;
            const mouseY = (d3.event.offsetY - transform.y) / transform.k;
            
            // Find node under cursor
            const node = simulation.nodes().find(n => {{
                const dx = n.x - mouseX;
                const dy = n.y - mouseY;
                return Math.sqrt(dx * dx + dy * dy) < 8;
            }});
            
            if (node) {{
                tooltip
                    .style('left', (d3.event.pageX + 10) + 'px')
                    .style('top', (d3.event.pageY - 28) + 'px')
                    .style('opacity', 0.9)
                    .html(`<strong>${{node.name}}</strong><br>${{node.url}}`);
            }} else {{
                tooltip.style('opacity', 0);
            }}
        }});
        
        // Filtering functionality
        let visibleNodes = rawNodes;
        let visibleLinks = rawLinks;
        let renderQuality = 'medium';
        
        // Convert raw data to proper format for D3
        const nodes = rawNodes.map(d => ({{...d}}));
        const links = rawLinks.map(d => ({{...d}}));
        
        // Set up domain filter
        const domainSelect = document.getElementById('domain-select');
        const uniqueDomains = [...new Set(nodes.map(n => n.domain))].sort();
        
        uniqueDomains.forEach(domain => {{
            const option = document.createElement('option');
            option.value = domain;
            option.textContent = domain;
            domainSelect.appendChild(option);
        }});
        
        domainSelect.addEventListener('change', e => {{
            const domain = e.target.value;
            if (domain === 'all') {{
                visibleNodes = nodes;
            }} else {{
                visibleNodes = nodes.filter(n => n.domain === domain);
            }}
            
            // Update links to only include connections between visible nodes
            const visibleNodeIds = new Set(visibleNodes.map(n => n.id));
            visibleLinks = links.filter(l => 
                visibleNodeIds.has(typeof l.source === 'object' ? l.source.id : l.source) && 
                visibleNodeIds.has(typeof l.target === 'object' ? l.target.id : l.target)
            );
            
            // Update counters
            document.getElementById('visible-nodes').textContent = visibleNodes.length;
            document.getElementById('visible-links').textContent = visibleLinks.length;
            
            // Restart simulation
            simulation.nodes(visibleNodes);
            simulation.force("link").links(visibleLinks);
            simulation.alpha(1).restart();
        }});
        
        // Set up render quality selector
        document.getElementById('render-quality').addEventListener('change', e => {{
            renderQuality = e.target.value;
            render();
        }});
        
        // Set up zoom buttons
        document.getElementById('zoom-in').addEventListener('click', () => {{
            d3.select(canvas).transition().duration(500).call(zoom.scaleBy, 1.5);
        }});
        
        document.getElementById('zoom-out').addEventListener('click', () => {{
            d3.select(canvas).transition().duration(500).call(zoom.scaleBy, 0.75);
        }});
        
        document.getElementById('reset').addEventListener('click', () => {{
            d3.select(canvas).transition().duration(500).call(zoom.transform, d3.zoomIdentity);
        }});
        
        // Initialize simulation
        simulation.nodes(nodes);
        simulation.force("link").links(links);
        
        // Update counters
        document.getElementById('visible-nodes').textContent = nodes.length;
        document.getElementById('visible-links').textContent = links.length;
        
        // Render function with performance optimizations
        function render() {{
            context.clearRect(0, 0, width, height);
            context.save();
            context.translate(transform.x, transform.y);
            context.scale(transform.k, transform.k);
            
            const renderLinks = renderQuality === 'low' && visibleLinks.length > 2000 
                ? visibleLinks.slice(0, 2000) 
                : visibleLinks;
                
            // Draw links
            context.strokeStyle = '#999';
            context.globalAlpha = 0.2;
            context.lineWidth = 0.5;
            
            for (const link of renderLinks) {{
                context.beginPath();
                const source = typeof link.source === 'object' ? link.source : simulation.nodes().find(n => n.id === link.source);
                const target = typeof link.target === 'object' ? link.target : simulation.nodes().find(n => n.id === link.target);
                
                if (source && target) {{
                    context.moveTo(source.x, source.y);
                    context.lineTo(target.x, target.y);
                    context.stroke();
                }}
            }}
            
            // Draw nodes
            context.globalAlpha = 1.0;
            
            for (const node of visibleNodes) {{
                context.beginPath();
                context.fillStyle = color(node.domain);
                context.arc(node.x, node.y, 5, 0, 2 * Math.PI);
                context.fill();
                
                if (renderQuality !== 'low') {{
                    context.strokeStyle = '#fff';
                    context.lineWidth = 1.5;
                    context.stroke();
                }}
            }}
            
            context.restore();
        }}
        
        // Start simulation
        simulation.on("tick", render);
        simulation.alpha(1).restart();
    </script>
</body>
</html>"###,
            nodes = nodes.join(","),
            links = links.join(","),
            node_limit = node_limit,
            total_nodes = total_nodes,
            total_links = links.len()
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

